# Observability and operating cost

## Prometheus and Grafana

The public `/metrics` endpoint performs one database aggregation per scrape. It does not call
the Kubernetes API once per workspace. The stable metric families are:

- `mwc_workspaces{state}` for lifecycle totals.
- `mwc_resource_requested{resource,unit}` for platform requested capacity.
- `mwc_user_workspaces{user_id,state}` for per-owner workspace totals.
- `mwc_user_resource_requested{user_id,resource,unit}` for per-owner requested capacity.
- `mwc_jobs`, `mwc_jobs_pending`, `mwc_http_requests_total`, and
  `mwc_configured_replicas` for control-plane operation.

User display names are deliberately excluded because `/metrics` is an operational endpoint and
must not become an unauthenticated identity directory. Grafana groups owners by stable user UUID.

Enable `monitoring.serviceMonitor.enabled` only when the Prometheus Operator CRDs are installed.
The optional ServiceMonitor scrapes the existing API Service on `/metrics`; it does not expose a
second public listener.

Actual CPU and memory remain Kubernetes runtime telemetry. Every managed Namespace, Pod, PVC,
and workload now carries installation, organization, owner-user, and workspace UUID labels.
Grafana can join `container_cpu_usage_seconds_total` or `container_memory_working_set_bytes` with
`kube_pod_labels` and aggregate on `label_workspace_memeloop_dev_owner_user_id`. This avoids
turning the MWC API into another metrics collector.

## Loki

MWC writes structured operational logs to stdout/stderr and never logs injection plaintext. A
cluster Promtail installation can retain these labels:

- `mwc_installation`
- `mwc_organization_id`
- `mwc_owner_user_id`
- `mwc_workspace_id`

They are identifiers, not credential values. Use Loki retention and tenant isolation appropriate
to the deployment; MWC does not embed or operate its own log store.

## Large public installations

Use PostgreSQL mode and multiple identical control-plane replicas. SQLite remains single-replica.
The browser uses one batch runtime request for the visible organization every 30 seconds and
pauses periodic refresh while the tab is hidden. Kubernetes Events are loaded only when a user
opens the event view. The batch API uses fixed-count cluster list operations rather than one
request per workspace.

Each workspace remains a separate Namespace and StatefulSet by design. Use the `standard`
runtime adaptation for low-cost workloads that do not require the BuildKit sidecar. ttyd has a
small explicit request and a bounded limit so scheduling and capacity accounting are predictable.
Do not share one workspace Pod between mutually untrusted users to reduce cost.
