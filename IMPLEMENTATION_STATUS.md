# Implementation status

This file is the durable continuation checkpoint for active implementation work. After context
compaction, continue from **Next action**; do not repeat the completed audits below unless a new
failure supplies contradictory evidence.

Last updated: 2026-09-06

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

Start the separately queued collision-free Kubernetes resource naming and migration design below.
Do not repeat the completed UI release audit unless new production evidence contradicts it.

### Separate migration constraint queued after this release

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
