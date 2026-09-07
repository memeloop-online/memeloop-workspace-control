# Implementation status

This is the durable continuation checkpoint. Continue from **Next actions** after context
compaction. Do not repeat completed audits unless new evidence contradicts them.

Last updated: 2026-09-07

## Active goal

Finish the single-model release, promote the verified development images, and leave production
with one collision-free resource layout across the database, Kubernetes, source, tests, docs, and
product terminology.

## Completed production migration

- The control plane is running schema 19. Its database stores only the dedicated Namespace scope
  and Namespace; every resource name and route is derived from the installation and workspace IDs.
- The installed binary exposes only `migrate`, `export`, `import`, and `migrate-to-postgres`
  database commands. The one-time transition command is absent.
- Four live workspace rows have collision-free runtime identities. All four Pods are Ready, use
  their exact canonical PVC, and retain their original SSH NodePort:
  - `maintainance`: `w-bd2dc9ca6aa2b1b5`, port `31871`, 2 GiB.
  - `tiddlywiki-dev`: `w-b268ff46894a14b9`, port `30953`, 30 GiB.
  - `game-forking`: `w-97645a0fb4771b1d`, port `30732`, 60 GiB.
  - `rust-dev-test`: `w-b405d2441a1ee149`, port `32671`, 100 GiB.
- Each workspace passed real public-key SSH command execution, one-time Web Shell access, PVC
  identity checks, host-key continuity, and a full restart with the same PVC and host key.
- Before copying `tiddlywiki-dev` and `game-forking`, the stopped source volumes were snapshotted.
  Target data was verified by a read-only content tree hash including paths, types, owners, modes,
  links, ACLs, extended attributes, and SELinux metadata. Timestamps were normalized. Regenerable
  caches, `node_modules`, and process sockets were intentionally excluded.
- Target cleanup reclaimed 1,133,764,608 bytes from `tiddlywiki-dev` and 526,131,200 bytes from
  `game-forking` in addition to their earlier cache allowlist cleanup. `.codex`, repositories,
  SSH/Git settings, and user files were preserved.
- After start/restart acceptance, the superseded Kubernetes objects, PVCs, PVs, Longhorn volumes,
  and temporary migration Jobs were removed. They are not recoverable from the removed source
  volumes. The healthy target volumes and these pre-start snapshots remain:
  - `mwc-target-b268ff46894a14b9-prestart-20260907`
  - `mwc-target-97645a0fb4771b1d-prestart-20260907`
- Encrypted mode-0600 database checkpoints are stored outside the repository at
  `.mwc-migration-backups/2026-09-07/control-plane-pre-v19.json` and
  `.mwc-migration-backups/2026-09-07/control-plane-v19.json`. Never print their contents.

## Current deployed release

- Product revision: `1576ff8cba941974fb0a8f8a2f12312da8a5e609`.
- GitHub Actions run `34144332446` passed the complete frontend, Rust, SQLite,
  PostgreSQL, bootstrap, Helm, image-contract, publication, and provenance suite.
- Control-plane image:
  `sha256:b81e128fed0a1291f266496658091f2e474f7af640e496a6838549256b57a42a`.
- ttyd image:
  `sha256:ff261f623020a2dd8b8a2d3a11d2e1a41e782bc560802cd3fae307c5a76aa75e`.
- Workspace-base image:
  `sha256:d1b86968c441b5b0ee17b1560d170d0246867892f9bc1ba8d82d1f870c6d0f2b`.
- GitOps commit `39aa3f2` pins the exact revision and image digests. Argo CD reports the
  application Synced/Healthy. Public `/metrics` returns 404; the internal 8081 endpoint serves
  OpenMetrics; `/livez` and `/readyz` return HTTP 200.
- The stable `workspace-data` claim-template name is deployed. PVC identity remains unique through
  the StatefulSet name (`workspace-data-w-<workspace-short-id>-0`), avoiding immutable updates and
  retaining existing data.

## Development images and templates

- `Maintenance`: `harbor.k3s.onetwo.website/library/cluster-admin:20260907-22c45c3` at
  `sha256:9b9f6cfc8bd2197fdc9a2e69ada3a8b7d0bdc129ba33908f584ce0e49a88b2e8`.
- `Node Dev`: `harbor.k3s.onetwo.website/library/node-dev:20260907-d866f4d` at
  `sha256:4b938d6210d5f2bebbd43d96c51d23d969ee0f47540d6e369931a1c756b2bc12`.
- `Rust Dev`: `harbor.k3s.onetwo.website/library/rust-dev:20260907-0416764` at
  `sha256:e71597592feacc4adb69643931af3bf4ee7f3f1060aac302c2a8f595a105a4b3`.
- Forgejo CI and real K3s pulls verified Rust 1.98.1, Cargo 1.98.1, Node 24.20.0,
  npm 11.19.0, Codex CLI 0.153.4, GitHub CLI 2.100.0, Helm 4.2.4, kubectl 1.36.4,
  OpenSSH, jq, clang, cmake, and the required browser/native build libraries.
- The three active templates and image policies now point to these immutable images. Three unused
  duplicate templates were disabled and deleted through the audited API.

## Source closeout completed

- The clean schema baseline initializes new databases directly at schema 19 and rejects older
  database versions with one generic unsupported-version error. Historical transformation SQL,
  compatibility branches, stale terminology, and the transition CLI are removed.
- A system-admin-only `PUT /api/v1/workspaces/{workspace_id}/image` endpoint safely upgrades a
  stopped workspace. It requires an enabled exact lowercase SHA-256 image policy, optimistic
  generation matching, an idempotency key, and atomically updates the template snapshot, image,
  generation, reconcile job, audit row, and event in SQLite or PostgreSQL.
- Independent review caught and fixed non-hex digest acceptance. Production implementation files
  are below the 400-line maintainability target. The final main-branch suite and tracked-tree scan
  for removed compatibility terminology pass.

## Active closeout

- `tiddlywiki-dev` and `game-forking` were stopped through the API, upgraded through the audited
  stopped-workspace image endpoint, and started on the latest Node/Rust and BuildKit images. Their
  canonical PVCs are unchanged. Both are Ready; Node/Codex/gh/Helm and retained `.codex` state were
  verified over host-key-checked SSH on `tiddlywiki-dev`.
- A real SSH check found that sshd's generated `SetEnv PATH` omits `/usr/local/cargo/bin`, although
  the Rust binaries are present in the image. A source fix and regression test are in progress.
- Codex reports a permission warning while pruning stale entries under its dedicated ephemeral
  scratch volume. Ownership is being checked separately from the persistent `.codex` session and
  log state; persistent `.codex` data must not be deleted or moved to an ephemeral volume.
- Another Codex task is actively using ports `31871` and `32671`. Normal `.codex` session, WAL,
  and log writes are expected and safe. Do not stop, restart, or switch those two workspaces until
  that task reports completion.
- The final GHCR workspace-base digest is enabled in the image policy. Three superseded
  workspace-base policies and six unreferenced old development-image policies are disabled. The
  two old policies still referenced by `maintainance` and `rust-dev-test` remain enabled until
  their coordinated upgrade.

## Next actions

1. Land and verify the generated SSH environment fix, publish the resulting control image, and
   deploy it through GitOps.
2. Restart only `tiddlywiki-dev` and `game-forking`, then verify Rust/Cargo PATH, Codex scratch
   permissions, ttyd, SSH, image IDs, and retained data.
3. Coordinate a short window for `maintainance` and `rust-dev-test`, which remain in active use by
   another Codex CLI task; promote and validate them without cleaning `.codex`.
4. Add the final workspace-base image policy and disable superseded policies only after no
   workspace references them.
5. Perform one final database/Kubernetes/source acceptance pass and update this checkpoint. Do not
   repeat completed migration, CI, observability, or terminology audits without new contrary
   evidence.

## Safety invariants

- Do not interrupt ports `31871` or `32671` until the borrowing task reports completion.
- Never expose tokens, private keys, decrypted credentials, certificates, or database snapshots.
- Workspace image changes require the stopped-state, exact-digest, policy, generation,
  idempotency, audit, and reconcile gates; do not edit SQLite directly.
- Do not delete a retained target PVC or pre-start snapshot during source closeout.
