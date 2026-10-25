#!/usr/bin/env node
// One-time offline SQLite upgrade. Run with Node 24, with all controllers stopped:
// node scripts/upgrade-runtime-events-v30-v31.ts --controller-stopped SOURCE.db NEW.db
// The source stays read-only. The target must not exist; inspect it before cutover.
import { DatabaseSync, backup } from 'node:sqlite';
import { open, unlink } from 'node:fs/promises';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

const emptyDetails = JSON.stringify({
  event_uid: null, pod_uid: null, reason: null, container: null,
  scheduling_blockers: [], diagnostic: null,
});

// This is the final v31 table, without the v30 category+second unique constraint.
const eventTable = "CREATE TABLE workspace_runtime_incidents (id TEXT PRIMARY KEY, installation_id TEXT NOT NULL, workspace_id TEXT NOT NULL, category TEXT NOT NULL CHECK (category IN ('ephemeral_storage', 'memory_pressure', 'pid_pressure', 'disk_pressure', 'evicted', 'temporary_storage_provisioning', 'temporary_storage_attachment', 'volume_unavailable', 'container_backoff', 'image_pull', 'scheduling', 'health_check', 'other')), observed_at BIGINT NOT NULL, count BIGINT NOT NULL CHECK (count > 0), first_seen_at BIGINT NOT NULL, last_seen_at BIGINT NOT NULL CHECK (last_seen_at >= first_seen_at), details_json TEXT NOT NULL, FOREIGN KEY (workspace_id) REFERENCES workspaces(id) ON DELETE CASCADE)";

function verifyIntegrity(db: DatabaseSync): void {
  const integrity = db.prepare('PRAGMA integrity_check').all();
  if (integrity.length !== 1 || integrity[0].integrity_check !== 'ok') {
    throw new Error('SQLite integrity_check failed');
  }
  if (db.prepare('PRAGMA foreign_key_check').all().length !== 0) {
    throw new Error('SQLite foreign_key_check failed');
  }
}

function counts(db: DatabaseSync): Map<string, number | bigint> {
  const tables = db.prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name").all();
  return new Map(tables.map(({ name }) => {
    const table = String(name).replaceAll('"', '""');
    const { count } = db.prepare(`SELECT COUNT(*) AS count FROM "${table}"`).get()!;
    return [String(name), count as number | bigint];
  }));
}

export async function upgrade(sourcePath: string, targetPath: string): Promise<void> {
  const source = new DatabaseSync(sourcePath, { readOnly: true });
  let target: DatabaseSync | undefined;
  let created = false;
  try {
    const version = source.prepare('SELECT MAX(version) AS version FROM schema_migrations').get()?.version;
    if (version !== 30) throw new Error('Source must have exactly current schema version 30');
    verifyIntegrity(source);
    const before = counts(source);
    // Exclusive creation prevents backup() from overwriting any existing file.
    const reserved = await open(targetPath, 'wx', 0o600);
    created = true;
    await reserved.close();
    await backup(source, targetPath);
    target = new DatabaseSync(targetPath);
    target.exec('PRAGMA foreign_keys = ON; BEGIN IMMEDIATE');
    target.exec('ALTER TABLE workspace_runtime_incidents RENAME TO workspace_runtime_incidents_v30');
    target.exec(eventTable);
    target.prepare('INSERT INTO workspace_runtime_incidents (id, installation_id, workspace_id, category, observed_at, count, first_seen_at, last_seen_at, details_json) SELECT id, installation_id, workspace_id, category, observed_at, count, first_seen_at, last_seen_at, ? FROM workspace_runtime_incidents_v30').run(emptyDetails);
    target.exec('DROP TABLE workspace_runtime_incidents_v30');
    target.exec('CREATE INDEX workspace_runtime_incidents_recent_idx ON workspace_runtime_incidents (installation_id, workspace_id, last_seen_at, observed_at)');
    target.prepare('INSERT INTO schema_migrations (version, applied_at) VALUES (31, ?)').run(Math.floor(Date.now() / 1000));
    const after = counts(target);
    if (before.size !== after.size) throw new Error('Table count changed');
    for (const [table, count] of before) {
      const expected = table === 'schema_migrations' ? Number(count) + 1 : count;
      if (after.get(table) !== expected) throw new Error(`Row count changed in ${table}`);
    }
    verifyIntegrity(target);
    target.exec('COMMIT');
    target.close();
    target = undefined;
  } catch (error) {
    target?.close();
    target = undefined;
    if (created) await unlink(targetPath);
    throw error;
  } finally {
    source.close();
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const [acknowledgement, source, target, ...extra] = process.argv.slice(2);
  if (acknowledgement !== '--controller-stopped' || !source || !target || extra.length) {
    throw new Error('Usage: node upgrade-runtime-events-v30-v31.ts --controller-stopped SOURCE.db NEW.db');
  }
  await upgrade(source, target);
  console.log('Created verified schema v31 target; source unchanged. Inspect target before cutover.');
}
