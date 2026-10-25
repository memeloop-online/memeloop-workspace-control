import { test } from 'node:test';
import assert from 'node:assert/strict';
import { DatabaseSync } from 'node:sqlite';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { upgrade } from './upgrade-runtime-events-v30-v31.ts';

const oldTable = "CREATE TABLE workspace_runtime_incidents (id TEXT PRIMARY KEY, installation_id TEXT NOT NULL, workspace_id TEXT NOT NULL, category TEXT NOT NULL CHECK (category IN ('ephemeral_storage', 'memory_pressure', 'pid_pressure', 'disk_pressure', 'evicted', 'temporary_storage_provisioning', 'temporary_storage_attachment', 'volume_unavailable', 'container_backoff', 'image_pull', 'scheduling', 'health_check', 'other')), observed_at BIGINT NOT NULL, count BIGINT NOT NULL CHECK (count > 0), first_seen_at BIGINT NOT NULL, last_seen_at BIGINT NOT NULL CHECK (last_seen_at >= first_seen_at), UNIQUE (installation_id, workspace_id, category, observed_at), FOREIGN KEY (workspace_id) REFERENCES workspaces(id) ON DELETE CASCADE)";

function fixture(path: string, version = 30): void {
  const db = new DatabaseSync(path);
  db.exec(`
    PRAGMA foreign_keys = ON;
    CREATE TABLE schema_migrations (version BIGINT PRIMARY KEY, applied_at BIGINT NOT NULL);
    INSERT INTO schema_migrations VALUES (${version}, 1);
    CREATE TABLE workspaces (id TEXT PRIMARY KEY, preserved TEXT);
    INSERT INTO workspaces VALUES ('workspace-1', 'unrelated workspace data');
    INSERT INTO workspaces VALUES ('workspace-2', 'another installation');
    ${oldTable};
    CREATE INDEX workspace_runtime_incidents_recent_idx ON workspace_runtime_incidents (installation_id, workspace_id, last_seen_at, observed_at);
    INSERT INTO workspace_runtime_incidents VALUES ('0199bb7c-8000-7000-8000-000000000001', 'install-1', 'workspace-1', 'ephemeral_storage', 100, 7, 100, 200);
    INSERT INTO workspace_runtime_incidents VALUES ('old-row-2', 'install-2', 'workspace-2', 'scheduling', 101, 3, 101, 201);
  `);
  db.close();
}

test('offline v30 backup upgrade preserves all original rows and IDs, adds empty facts, and removes second-level uniqueness', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'mwc-v31-'));
  const source = join(dir, 'source.db');
  const target = join(dir, 'v31.db');
  try {
    fixture(source);
    const originalBytes = await readFile(source);
    const original = new DatabaseSync(source, { readOnly: true });
    const originalRows = original.prepare('SELECT * FROM workspace_runtime_incidents ORDER BY id').all();
    const originalParents = original.prepare('SELECT * FROM workspaces ORDER BY id').all();
    original.close();
    await upgrade(source, target);
    assert.deepEqual(await readFile(source), originalBytes);
    const upgraded = new DatabaseSync(target);
    try {
      assert.equal(upgraded.prepare('SELECT MAX(version) AS version FROM schema_migrations').get()!.version, 31);
      const rows = upgraded.prepare('SELECT * FROM workspace_runtime_incidents ORDER BY id').all();
      assert.deepEqual(rows.map(({ details_json, ...old }) => {
        assert.deepEqual(JSON.parse(String(details_json)), {
          event_uid: null, pod_uid: null, reason: null, container: null,
          scheduling_blockers: [], diagnostic: null,
        });
        return old;
      }), originalRows.map(row => ({ ...row })));
      assert.deepEqual(upgraded.prepare('SELECT * FROM workspaces ORDER BY id').all(), originalParents);
      // Compare the operational tool to the exact fresh-database v31 schema.
      const rustSchema = await readFile(new URL('../src/storage/schema.rs', import.meta.url), 'utf8');
      const table = rustSchema.split('\n').find(line => line.includes('CREATE TABLE workspace_runtime_incidents '))!.trim().slice(1, -2);
      assert.equal(upgraded.prepare("SELECT sql FROM sqlite_master WHERE name = 'workspace_runtime_incidents'").get()!.sql, table);
      upgraded.prepare('INSERT INTO workspace_runtime_incidents SELECT ?, installation_id, workspace_id, category, observed_at, count, first_seen_at, last_seen_at, details_json FROM workspace_runtime_incidents WHERE id = ?').run('new-event-uid', '0199bb7c-8000-7000-8000-000000000001');
      assert.equal(upgraded.prepare('SELECT COUNT(*) AS count FROM workspace_runtime_incidents').get()!.count, 3);
      assert.equal(upgraded.prepare('PRAGMA integrity_check').get()!.integrity_check, 'ok');
      assert.deepEqual(upgraded.prepare('PRAGMA foreign_key_check').all(), []);
    } finally { upgraded.close(); }
  } finally { await rm(dir, { recursive: true, force: true }); }
});

test('upgrade rejects non-v30 sources and never overwrites an existing target', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'mwc-v31-reject-'));
  try {
    const source = join(dir, 'source.db');
    const target = join(dir, 'target.db');
    fixture(source, 29);
    await assert.rejects(upgrade(source, target), /schema version 30/);
    await assert.rejects(readFile(target), { code: 'ENOENT' });
    await rm(source);
    fixture(source);
    await writeFile(target, 'preserve existing destination');
    await assert.rejects(upgrade(source, target), { code: 'EEXIST' });
    assert.equal(await readFile(target, 'utf8'), 'preserve existing destination');
  } finally { await rm(dir, { recursive: true, force: true }); }
});

test('backup includes committed source WAL without changing source main or WAL bytes', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'mwc-v31-wal-'));
  const source = join(dir, 'source.db');
  const target = join(dir, 'target.db');
  let keeper: DatabaseSync | undefined;
  try {
    fixture(source);
    keeper = new DatabaseSync(source);
    keeper.exec(`
      PRAGMA journal_mode = WAL;
      PRAGMA wal_autocheckpoint = 0;
      INSERT INTO workspace_runtime_incidents VALUES ('wal-event', 'install-1', 'workspace-1', 'other', 202, 2, 202, 203);
      UPDATE workspaces SET preserved = 'committed WAL workspace data' WHERE id = 'workspace-1';
    `);
    // No controller/writer is running. Retain this idle connection only to
    // prevent SQLite's last-connection checkpoint from removing the fixture WAL.
    const sourceBytes = await readFile(source);
    const walBytes = await readFile(`${source}-wal`);
    assert.ok(walBytes.length > 0);
    await upgrade(source, target);
    assert.deepEqual(await readFile(source), sourceBytes);
    assert.deepEqual(await readFile(`${source}-wal`), walBytes);
    // The completed target is standalone; no target sidecars are needed.
    await assert.rejects(readFile(`${target}-wal`), { code: 'ENOENT' });
    await assert.rejects(readFile(`${target}-shm`), { code: 'ENOENT' });
    const upgraded = new DatabaseSync(target, { readOnly: true });
    try {
      assert.equal(upgraded.prepare("SELECT count FROM workspace_runtime_incidents WHERE id = 'wal-event'").get()!.count, 2);
      assert.equal(upgraded.prepare("SELECT preserved FROM workspaces WHERE id = 'workspace-1'").get()!.preserved, 'committed WAL workspace data');
    } finally { upgraded.close(); }
  } finally {
    keeper?.close();
    await rm(dir, { recursive: true, force: true });
  }
});
