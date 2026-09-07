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

- Product revision: `51bb054692c848e7cdb4a86cf2346f6f7b2a0a47`.
- GitHub Actions: run `34121291689`, all verification and four image publication jobs passed.
- Control-plane image:
  `sha256:a85e7946c9485cb78d30c8edc2342e376aa20924a512910268b2f041b9c8d180`.
- ttyd image:
  `sha256:59ceec062c34180f6dd31efb7db2de5b096e01449121f190c1a9bafc005d7a7b`.
- GitOps commit `ed7b93eb16bfd0fce5bd3299a45c93f98c11d7fb` pins the exact product revision
  and image digests. Argo CD is Synced/Healthy; `/livez` and `/readyz` return HTTP 200.
- Post-deploy encrypted export confirms schema 19 and four workspace rows without transitional
  runtime columns.

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

## Source closeout in progress

- The clean schema baseline initializes new databases directly at schema 19 and rejects older
  database versions with one generic unsupported-version error. Historical transformation SQL,
  compatibility branches, stale terminology, and the transition CLI are removed.
- A system-admin-only `PUT /api/v1/workspaces/{workspace_id}/image` endpoint safely upgrades a
  stopped workspace. It requires an enabled exact lowercase SHA-256 image policy, optimistic
  generation matching, an idempotency key, and atomically updates the template snapshot, image,
  generation, reconcile job, audit row, and event in SQLite or PostgreSQL.
- Independent review caught and fixed non-hex digest acceptance. Production implementation files
  are below the 400-line maintainability target. Local web tests, focused Rust tests, formatting,
  and strict Clippy pass; final main-branch integration tests are running.

## Next actions

1. Finish main-branch integration tests and enforce a zero-match tracked-tree scan for removed
   compatibility terminology.
2. Push the clean final source, require the complete GitHub Actions suite, and publish four new
   immutable images.
3. Update GitOps to the clean final revision/digests and require Argo Synced/Healthy plus schema,
   health, metrics, and log checks.
4. Use the audited stopped-workspace image endpoint to promote `tiddlywiki-dev` and
   `game-forking`, then validate image IDs, BuildKit 0.33.0, ttyd, SSH, Web Shell, restart, and
   persisted data.
5. Coordinate a short window for `maintainance` and `rust-dev-test`, which are currently available
   to another Codex CLI test task, then promote and validate them the same way.
6. Disable superseded image policies after no workspace references those images, and perform the
   final database/Kubernetes/source scan.

## Safety invariants

- Do not interrupt ports `31871` or `32671` until the borrowing task reports completion.
- Never expose tokens, private keys, decrypted credentials, certificates, or database snapshots.
- Workspace image changes require the stopped-state, exact-digest, policy, generation,
  idempotency, audit, and reconcile gates; do not edit SQLite directly.
- Do not delete a retained target PVC or pre-start snapshot during source closeout.
