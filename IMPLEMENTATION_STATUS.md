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

## In progress

- No product implementation item remains open from this checkpoint. Only the time-based API-key
  safety closeout below remains.

## Next action

1. After the API-key overlap window, compare historical-key `last_used_at` values with the recorded
   rotation checkpoint.
2. Revoke only keys that remained unused, then verify daily clients with the replacement keys.

## Deferred safety closeout

- Observe the retired API keys for 24–72 hours and revoke only after their `last_used_at` values stop
  advancing. Do not revoke the unrelated acceptance administrator key without transferring its
  organization ownership first.
