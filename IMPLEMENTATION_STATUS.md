# Implementation status

This file is the durable continuation checkpoint for active implementation work. After context
compaction, continue from **Next action**; do not repeat the completed audits below unless a new
failure supplies contradictory evidence.

Last updated: 2026-09-03

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
- Daily-use API tokens were rotated to explicit scopes and expiry. Historical keys remain only for
  a deliberate no-downtime observation window.
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

## In progress

- Observe the historical API-key overlap window until at least 24 hours have elapsed from the
  recorded rotation checkpoint. No implementation or deployment work remains for the twelve
  2026-09-03 fixes.

## Next action

1. After the API-key overlap window, compare historical-key `last_used_at` values with the recorded
   rotation checkpoint.
2. Revoke only keys that remained unused, then verify daily clients with the replacement keys.

## Deferred safety closeout

- Observe the retired API keys for 24–72 hours and revoke only after their `last_used_at` values stop
  advancing. Do not revoke the unrelated acceptance administrator key without transferring its
  organization ownership first.

## Operational access checkpoint

- The dedicated local Kubernetes identity still authenticates, but its current RBAC grants only
  Longhorn snapshot/volume and PersistentVolume operations, not Argo CD, Pod, or StatefulSet reads.
  GitOps and external service evidence are complete; direct in-cluster rollout evidence requires a
  reviewed read-only RBAC expansion. Never copy or disclose the private key.
