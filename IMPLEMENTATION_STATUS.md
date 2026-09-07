# Implementation status

This file is the durable continuation checkpoint for active implementation work. After context
compaction, continue from **Next action**; do not repeat the completed audits below unless a new
failure supplies contradictory evidence.

Last updated: 2026-09-07

## Active goal: complete canonical runtime migration

The user approved the production migration on 2026-09-07. Completion means the live database,
Kubernetes resources, source tree, tests, documentation, and product terminology use one runtime
naming model and contain no compatibility scheme or old generic workspace object names.

- [x] Export a mode-0600 encrypted schema-v17 control-plane snapshot and inventory all four live
  workspace rows, Namespaces, PVC/PV bindings, capacities, images, and running workloads.
- [x] Publish and deploy a bounded transition release that can atomically move one stopped
  workspace row to canonical runtime identity without exposing sensitive database values.
- [ ] Migrate one workspace at a time: stop; prove Pod absence; create the canonical PVC; copy and
  verify data; remove the old selector-owning StatefulSet; switch database identity; reconcile;
  start; verify SSH, Web Shell, storage, injections, and runtime telemetry.
- [ ] Remove old Kubernetes objects and PVCs only after each canonical workload and retained backup
  passes its rollback gate.
- [ ] Replace the transition release with the final single-model schema and code, remove
  compatibility branches, triggers, and fields, and make a case-insensitive source-tree scan a CI
  failure.
- [ ] Publish the final four images, update GitOps, verify Argo health and production behavior, then
  record all revisions, digests, backup identifiers, and migration evidence here.

Current production checkpoint (continue here after compaction; do not repeat earlier evidence):

- Transition product revision `9e29a3806c008392a46cf6753c975e73574bc43a` passed GitHub Actions
  run `34077513206`; the deployed control-plane image digest is
  `sha256:3470086bc5ff9634437ecffc56e779a2c0c7b819b9d694fec4cf3e96cf48463d`.
- GitOps revision `08bac4ca5722c22495c305fdb1c45297898d9099` declares that exact revision and
  digest. Argo source refresh was temporarily blocked by repo-server GitHub transport failures; the
  live StatefulSet was set to the already-committed immutable digest and is Ready.
- `maintainance` (`01a06170-582c-70f0-bd2d-c9ca6aa2b1b5`) is fully canonical. It uses
  `w-bd2dc9ca6aa2b1b5`, PVC `workspace-data-w-bd2dc9ca6aa2b1b5-0`, and preserved NodePort
  `31871`. File manifests matched before cutover; SSH command execution, the one-time Web Shell URL,
  restart persistence, and host-key continuity passed. All old generic objects and the source PVC
  were removed after those gates; the canonical workload remained Ready.
- `tiddlywiki-dev` (`01a06174-8ce2-7d52-b268-ff46894a14b9`) is stopped with no workspace Pod.
  Longhorn snapshot `mwc-canon-b268ff46894a14b9-20260907` is ready. Target PVC
  `workspace-data-w-b268ff46894a14b9-0` is Bound. The copy finished with equal entry counts. The
  first archive check differed only because the two filesystem mount roots have different mtimes;
  read-only Job `mwc-verify-tree-b268ff46894a14b9` now hashes the complete tree below the mount
  root, including file data, ownership, mode, links, ACLs and xattrs. The source PVC, old
  StatefulSet, NodePort `30953`, and database runtime row remain unchanged until it passes.
- `game-forking` (`01a06180-f1c9-7a01-9764-5a0fb4771b1d`) is stopped with no workspace Pod.
  Snapshot `mwc-canon-97645a0fb4771b1d-20260907` and canonical 60-GiB target PVC
  `workspace-data-w-97645a0fb4771b1d-0` are ready. Its source-side tree hash is complete; the
  target-side reader is paused in place while tiddlywiki receives the Longhorn read bandwidth.
  Its old StatefulSet, source PVC, database row and NodePort `30732` remain unchanged.
- `rust-dev-test` (`01a06180-f652-7cd3-b405-d2441a1ee149`) is fully canonical and Ready on the
  retained original Longhorn volume under PVC `workspace-data-w-b405d2441a1ee149-0`. SSH, Web
  Shell, NodePort `32671`, host-key continuity and restart persistence passed; old generic objects
  were removed.
- The v19 transition source is locally complete through `086b36a`: canonical names are derived,
  redundant runtime columns and `runtime_profile` are dropped, the one-time transition command is
  removed, and BuildKit is pinned to verified v0.33.0 digest
  `sha256:80b15f0735e87bab7bf59ec4d695dfb4a7cfb25521cf56dc75d6f256285b63ef`.
- Toolchain images passed internal Forgejo CI and are immutable in Harbor: Rust commit `0416764`
  digest `sha256:e71597592feacc4adb69643931af3bf4ee7f3f1060aac302c2a8f595a105a4b3`, Node commit
  `d866f4d` digest `sha256:4b938d6210d5f2bebbd43d96c51d23d969ee0f47540d6e369931a1c756b2bc12`, and maintenance
  commit `22c45c3` digest `sha256:9b9f6cfc8bd2197fdc9a2e69ada3a8b7d0bdc129ba33908f584ce0e49a88b2e8`.
  The full MWC canary and template/workspace promotion remain gated on PVC cutover and exact
  in-cluster image-contract checks.

Safety invariants:

- The old and canonical StatefulSets must never coexist because their selectors overlap.
- The source PVC is never deleted before the target workload passes data and connection checks.
- Control-plane SQLite writer fencing is required for any offline database operation.
- Secret, token, certificate, and decrypted injection values must never enter logs or this file.

## Completed goal: collision-free runtime identity and migration safety

This implementation phase is complete. Do not repeat the completed UI, production, naming, or
migration-safety audits below unless new evidence contradicts them. GitOps and the cluster remain
out of scope until a separate migration window is approved.

- [x] Persist immutable `legacy_v1 | prefixed_v2` runtime identity and centralize Kubernetes object,
  PVC, SSH and Web Shell names.
- [x] Preserve the five existing dedicated Namespace/`workspace-data-workspace-0` layouts as
  `legacy_v1`; no existing claim has been renamed, adopted, deleted or moved.
- [x] Add collision-free shared-Namespace manifests, selectors, routing, runtime telemetry keys and
  ownership-checked staged deletion.
- [x] Close independent-review findings: Pod/PVC deletion reference guard, installation-bound
  identity decode, PostgreSQL old-writer compatibility, transactional snapshot validation, and
  shared-Namespace creation preflight.
- [x] Close Chart/runbook findings for kube-state-metrics prerequisites, immutable StatefulSet
  claim-template transitions, and the three distinct PVC reuse/relocation paths.
- [x] Run formatting, both strict Clippy gates, focused runtime/schema/snapshot/resource tests,
  ShellCheck, K3s script tests, Helm lint, and Prometheus rule validation locally. Three independent
  release reviews report no remaining P1/P2 findings.
- [x] Use GitHub Actions for the complete SQLite/PostgreSQL/Helm/image verification to avoid
  rebuilding the full suite locally.
- [x] Record the upstream revision and CI evidence here. Do not deploy this naming change or edit
  GitOps as part of this phase.

### Release evidence

- Upstream revision `40f3c26219af3f98f25474235ce0f54b2b1bee6b` passed GitHub Actions run
  `34054916210`. The run covered the web checks and build, Rust 1.98 formatting, both strict Clippy
  gates, the complete SQLite and PostgreSQL test suites, full-Home bootstrap recovery, Helm and
  Prometheus validation, runtime image checks, all four image builds, and provenance attestations.
- Published immutable image manifests:
  - control plane: `ghcr.io/memeloop-online/memeloop-workspace-control:sha-40f3c26@sha256:c91d79526454e708e42473916645f3165ea345cbe9edca02c37d5af12d4274f6`
  - workspace: `ghcr.io/memeloop-online/memeloop-workspace-control-workspace:sha-40f3c26@sha256:98dc5286d5b954f8bc49d778ed32796caf94e8419e101f53ea1ce0d0155bcfd0`
  - ttyd: `ghcr.io/memeloop-online/memeloop-workspace-control-ttyd:sha-40f3c26@sha256:ed48ad9140a2dc1f012b82c374f409bc3ef9ecf5d6b0a30c5a7128c9afd6828a`
  - SSH jump host: `ghcr.io/memeloop-online/memeloop-workspace-control-ssh-jump:sha-40f3c26@sha256:d5c368740da7304fb1ce9b254c134453a64aa29fe5d3de800d32e10c366fd920`
- No GitOps repository, Kubernetes object, existing workspace PVC, or control-plane SQLite volume
  was changed during this phase. Deployment and data migration require a separately approved,
  fenced migration window.

## Completed goal: product-wide responsive UI closeout

Do not repeat the 2026-09-03 operational audit. The active worktree is the authority for this
goal. Current verified state:

- Workspace, settings, and audit React pages have been split into reusable components; the main
  page modules are below the repository's maintainability limit.
- Workspace stop/restart/delete use the shared application dialog. Stop/restart skip confirmation
  only when complete live telemetry reports exactly zero CPU; unknown telemetry still confirms.
- Stopped workspaces expose neither port mappings nor live Pod/container observations. The runtime
  API also filters stopped, deleting, completed, and terminating Pods while retaining event history.
- Workspace list statistics come from a database-wide filtered summary rather than the current
  cursor page. CPU, memory, and disk allocations render against organization quotas as accessible
  progress backgrounds with precise hover text.
- API-key permissions use responsive explanatory cards. Wildcard authorization and all `Legacy
  key` / `legacy` / unbounded-key compatibility are removed; schema v17 deletes such rows instead
  of converting them to another full-access form.
- Audit action labels cover current mutable resources. Future action codes render as diagnosable
  “other API operation” text rather than implying missing authentication. The API still requires a
  bearer key; current audit rows identify the account but do not yet identify the calling client.
- First and second 360/768/1024/1440 screenshot reviews were completed. The second review verified
  stopped-state controls, global summaries, single-scroll workspace layout, and the new key UI, and
  found two final defects: a 1024 audit-filter overflow and a 360 fixed-nav overlay. Both are fixed.
  Final targeted measurements are 360/360 for workspace and settings document/client width and
  1024/1024 for audit; the mobile Stop action opened the application dialog within three seconds.
- Web typecheck, 54 tests, and production build pass. Rust formatting, both strict Clippy gates,
  all 92 unit tests, every integration suite, and doc tests pass locally. The only remaining local
  maintainability issue was resolved by moving migration coordination out of `storage.rs`, reducing
  it from 568 to 166 lines; the new single-purpose migration module is 362 lines.

### Release evidence

- Product revision `c31891626c563f846f7aaf3688488fc5508dd78b` passed GitHub Actions run
  `34041789734`. The run covered the web typecheck/tests/build, Rust 1.98 formatting and both
  strict Clippy gates, the complete SQLite and PostgreSQL test suites, the full-Home bootstrap
  check, Helm verification, all four image builds, and build-provenance attestations.
- The published control-plane image is pinned by manifest digest
  `sha256:af4a9f4b01f9536422ccce81e4986e543d26a19a677237588ec03e079a9c5fe4`.
- GitOps revision `0560a99d0c60734f75ef419db89a461320ad6373` pairs that exact source revision
  and digest while preserving the Harbor proxy-cache repository and every existing production
  value. The new chart passed an offline lint and full render with the production values before
  the change was pushed.
- The single-replica rollout completed after its expected brief 503 window. Three consecutive
  post-rollout checks returned HTTP 200 for both `/livez` and `/readyz`; the application serves
  `assets/index-BswvaKCJ.js` and `assets/index-I9HBbLn9.css`, and a complete unauthenticated
  workspace API request returns HTTP 401.
- The final independent responsive review used 360 px workspace/settings views and a 1024 px
  audit view. It measured no horizontal overflow, confirmed the mobile navigation no longer
  overlays content, and opened the product confirmation dialog from the mobile Stop action. No
  release-blocking visual defect remained.

### Next action

The separately queued collision-free Kubernetes resource naming and migration design is complete
and recorded at the top of this file. Do not repeat the completed UI release audit unless new
production evidence contradicts it.

### Migration constraints carried into the completed naming release

- Existing workspace PVCs remain in their current dedicated Namespaces and must not be deleted.
- Shared-Namespace support first requires persisted `legacy_v1 | prefixed_v2` runtime identity and
  centralized collision-free names for every Kubernetes object, selector, route, and PVC reference.
- A control-plane SQLite Namespace move requires a stopped, integrity-checked volume migration or
  backup/restore; it is not a live file copy. No GitOps or cluster change is authorized for this
  design stage.

## Completed evidence

- The original architecture plan is recorded in `PLAN.md`.
- The twelve 2026-09-03 product fixes have been implemented and previously audited; the only
  remaining operational work is listed below.
- Atomic organization/user/workspace injection batch deletion is implemented and covered by
  SQLite and PostgreSQL rollback, idempotency, and reconcile-deduplication tests.
- Product revision `c0bd7407ffa9aecad66754628e1d2b10c864673e` passed GitHub Actions run
  `33718333511`, including web tests, strict Clippy, full Rust tests, PostgreSQL tests, ENOSPC
  bootstrap recovery, Helm validation, and image publication.
- Control-plane image digest
  `sha256:2d664b269642ea6b3e028b49b1ac6e36e791ffd553f63c2627dc919af463f52b` is deployed by
  GitOps revision `2188b38`; Argo CD is Synced/Healthy and the replacement pod is Ready with zero
  restarts.
- `/livez`, `/readyz`, and authenticated `/metrics` return HTTP 200 after the rollout.
- The 31 redundant cache/toolchain-default organization injections were deleted in one atomic
  request after asserting their exact count and that every candidate was unlocked, non-sensitive,
  and an environment variable. The exact seven required overrides remain.
- The batch reconcile queue drained to zero; all five workspace pods are Running and Ready with
  zero container restarts.
- Targeted production-UI regression passed for injection search clearing and deselection, loading
  skeletons, organization selection guards, upload-only avatars, and consistent resource meters.
- Authenticated runtime telemetry returned `available` storage samples and live used/capacity bytes
  for all five workspaces. Prometheus scrape/rules and the Grafana Home-usage panel are healthy.
- Web shell returned HTTP 200 and completed a real WebSocket open. SSH reached the public-key
  authentication boundary; the current machine has no matching private key for a full login test.
- Port-mapping revision `f94c2128330d7086290007e7454960fc3261f38f` passed GitHub Actions run
  `33730713365` and is deployed by GitOps revision `f980934` at control-plane digest
  `sha256:c1e9bd1f187f86d5640ce4f590dfb7b0947f554b1c3627c41070fcd7097159cd`.
- A real mapped port-3000 test returned a 303 bootstrap response with a session cookie and then
  HTTP 200 from the workspace process. An anonymous request and consumed-ticket replay were
  refused by the gateway. All test mappings, generated resources, and temporary processes were
  removed; the test workspace replacement Pod returned 3/3 Ready.
- Daily-use API tokens were rotated to explicit scopes and expiry before the current schema-v17
  removal of wildcard and unbounded historical keys.
- System administrators can now page through another user's API-key summaries and force-revoke a
  target key without exposing token or hash material. Cross-user revocation requires both
  `manage_system` and `manage_api_keys`, requires an audited reason, is idempotent, and commits the
  revocation and audit event atomically. The administrator endpoint rejects self-targeting so it
  cannot bypass personal recovery-key protection.
- Personal revocation now preserves the last usable `manage_api_keys` key and, for system
  administrators, the last usable `manage_system` key. Expired keys remain removable. The user
  directory provides paginated administrator controls with explicit active/expired/revoked states.
- Independent backend and UI reviews were applied: administrator self-targeting is rejected at
  both the HTTP and storage boundaries; the dialog cannot close mid-revocation, distinguishes load
  failures from empty results, supports retry and page fallback, and keeps a successful revocation
  visible even if the follow-up refresh fails.
- Local validation for this change passed 52 web tests and production UI build, 87 Rust unit tests,
  the complete Rust integration suite, strict all-target Clippy, and the repository's additional
  maintainability Clippy gates. PostgreSQL-specific revocation and listing coverage is included for
  CI where `MWC_TEST_POSTGRES_URL` is available.
- API-key management revision `29a74f4eb255672429ee622b02d6be43e6160546` was pushed. CI run
  `33759962378` passed the web, formatting, maintainability, and strict Clippy gates, then exposed a
  PostgreSQL test-fixture collision before image publication. The administrator-key and scale-out
  PostgreSQL tests now each use a unique temporary schema, including cleanup, so installation
  identity and job rows cannot leak between the suite or concurrent CI shards.
- PostgreSQL-isolated revision `6402b421a5d33ee5db4359ad9a105a3f78a1f0ba` passed push CI run
  `33761599812`, then explicit publication run `33762304948` passed the same full verification and
  published all four GHCR images with provenance. The control-plane digest is
  `sha256:d58538eba4444179152171362eefe8073bd84e50448665c789a98ddc4f9318ad`.
- GitOps revision `adb347b` pins source revision `6402b421a5d33ee5db4359ad9a105a3f78a1f0ba`
  and the exact control-plane digest above. Production recovered from the single-replica rollout
  window with `/livez` and `/readyz` returning HTTP 200, served the revision's
  `assets/index-CW5kKWYO.js`, and returned HTTP 401 from the new administrator API-key route when
  called without authentication.
- Loki ingestion was verified through Grafana using aggregate-only LogQL queries: the six-hour
  result contained the MWC control-plane container and workspace containers without reading or
  printing log bodies. The existing dashboard expected datasource UID `loki`, while the datasource
  had an auto-generated UID. GitOps revisions `597b73d` and `614cf67` completed a two-stage
  migration to one persistent `uid=loki` datasource and removed the one-time delete directive.
  Grafana health is HTTP 200, the live dashboard and datasource use the same UID, and the dashboard
  LogQL selector is `{mwc_installation="k3si-7032544955"}`.
- All twelve fixes requested on 2026-09-03 were implemented and deployed with the evidence above.
  The active responsive-UI/schema-v17 release is tracked separately at the top of this file.

## Operational access checkpoint

- The dedicated local Kubernetes identity still authenticates, but its current RBAC grants only
  Longhorn snapshot/volume and PersistentVolume operations, not Argo CD, Pod, or StatefulSet reads.
  GitOps and external service evidence are complete; direct in-cluster rollout evidence requires a
  reviewed read-only RBAC expansion. Never copy or disclose the private key.
