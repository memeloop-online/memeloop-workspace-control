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
- Daily-use API tokens were rotated to explicit scopes and expiry. Historical keys remain only for
  a deliberate no-downtime observation window.

## In progress

- Port mapping creation reached `ready`, but the one-time bootstrap URL reset the client connection.
  The test mapping, Kubernetes resources, and temporary port-3000 process were fully removed. Root
  cause diagnosis is in progress; this is the only confirmed product regression in this checkpoint.

## Next action

1. Diagnose the port-mapping bootstrap connection reset from live routing evidence and code review.
2. Implement the smallest confirmed fix and run focused plus required release tests.
3. Re-deploy, repeat the real port-3000 access test, record evidence here, and keep only the API-key
   observation window open.

## Deferred safety closeout

- Observe the retired API keys for 24–72 hours and revoke only after their `last_used_at` values stop
  advancing. Do not revoke the unrelated acceptance administrator key without transferring its
  organization ownership first.
