# Observability and production diagnostics

## Health contracts

The public and internal listeners expose the same Kubernetes probe contracts:

- `GET /livez` returns `200` while the process and Tokio runtime can serve HTTP. It does not call
  the database, Kubernetes, Prometheus, Webhooks, or plugins. `/healthz` remains as a compatibility
  alias.
- `GET /readyz` performs a database ping with a two-second timeout. It returns `503` when the
  authoritative database cannot be reached. Kubernetes or Prometheus degradation does not remove
  the control plane from service, so operators can still inspect state and perform cleanup.

The Helm chart uses `/livez` for liveness and `/readyz` for readiness.

## OpenMetrics and Grafana

`GET /metrics` returns OpenMetrics 1.0 text and terminates with `# EOF`. A scrape performs one job
count query and one workspace aggregation; it does not call the Kubernetes API once per workspace.
The optional ServiceMonitor scrapes the existing internal Service on port `8081`. It creates no
NodePort, hostPort, extra listener, or public route. When NetworkPolicy is enabled, only the
configured `monitoring.serviceMonitor.namespace` is added as a scrape source.

Stable metric families include:

- `mwc_http_requests_total{method,route,status_class}` and
  `mwc_http_request_duration_seconds{method,route}` for request rate, errors and latency.
- `mwc_http_requests_active`, `mwc_streams_active{kind="sse"}`, and
  `mwc_http_body_declared_bytes_total` for active work, long-lived streams and bounded body volume.
- `mwc_upstream_requests_active{upstream}`, `mwc_upstream_requests_total{upstream,result}`, and
  `mwc_upstream_configured{upstream}` for Kubernetes, Prometheus and Webhook state.
- `mwc_jobs{status}` and `mwc_jobs_pending` for the durable background queue. The `status` values
  are `pending`, `running`, `completed`, and `failed`; a job enters `failed` after the ten-attempt
  retry limit and remains visible for operator review.
- `mwc_jobs_oldest_pending_age_seconds` and `mwc_jobs_max_active_attempts` for queue wait time
  and retry pressure. The first gauge is zero when the queue is empty.
- `mwc_process_resident_memory_bytes`, `mwc_process_virtual_memory_bytes`,
  `mwc_process_threads`, and `mwc_process_uptime_seconds` for process state.
- `mwc_allocator_bytes{state}` for jemalloc allocated, active, resident, mapped, metadata and
  retained bytes.
- `mwc_memory_component_bytes{component}` and `mwc_memory_limit_bytes{component}` for current
  request-buffer, queued SSE-event and plugin-registry estimates plus configured
  plugin/Prometheus buffer ceilings. Background jobs stay in the database rather than an
  in-process archive queue, so their depth is reported by `mwc_jobs` instead of a memory gauge.
- `mwc_plugins{state}`, `mwc_plugin_executions_active`, and `mwc_plugin_execution_limit` for the
  hot-reloaded Wasm registry and its bounded execution pool.
- `mwc_workspaces{state}`, `mwc_resource_requested{resource,unit}`,
  `mwc_user_workspaces{user_id,state}`, and
  `mwc_user_resource_requested{user_id,resource,unit}` for platform and owner allocation.

HTTP route labels use Axum route templates rather than concrete UUID paths. Unknown methods,
unmatched API paths, UI assets, status classes, upstream names, plugin states and memory components
all collapse into fixed vocabularies. User display names, workspace names, URLs, plugin IDs, error
messages and credential values are excluded. Per-owner series use stable user UUIDs because the
product explicitly supports Grafana aggregation per user.

The chart's optional `PrometheusRule` turns these signals into operational alerts. Workspace Home
PVC usage and node ephemeral-storage requests use warning/critical bands at 80%/90%. A sustained
oldest pending job age above 15 minutes raises `MwcJobsPendingTooOld`; one or more failed jobs
raises `MwcJobsFailed` after 10 minutes. The `monitoring.prometheusRule.warningFor` and
`monitoring.prometheusRule.criticalFor` values control storage alert duration (15 minutes and
5 minutes by default). Alert labels and routing remain under the existing Prometheus Operator
and Alertmanager configuration.

Actual workspace CPU and memory remain Kubernetes runtime telemetry. Managed Namespaces, Pods,
PVCs and workloads carry installation, organization, owner and workspace labels. Grafana can join
`container_cpu_usage_seconds_total` or `container_memory_working_set_bytes` with `kube_pod_labels`
and aggregate on `label_workspace_memeloop_dev_owner_user_id`.

## Release profiling

Linux release builds use jemalloc with heap-profiling support compiled in and retain the native
symbol table required for useful pprof captures. Sampling stays inactive unless
`MWC_DIAGNOSTICS_ENABLED=true`. The Helm equivalent is
`monitoring.diagnostics.enabled=true`; it also provides a 32 MiB memory-backed temporary directory
for heap dumps. The sampling interval is one sample per approximately 512 KiB of allocations.

Diagnostics exist only on the existing internal listener (`8081`) and require the internal Bearer
token even when the caller is already inside the cluster:

- `GET /diagnostics/process` returns process and allocator counters as JSON.
- `GET /debug/pprof/profile?seconds=10` captures 1–30 seconds of CPU samples and returns
  `mwc-cpu.pb`. Only one CPU capture can run per replica.
- `GET /debug/pprof/heap` returns the current sampled live heap as `mwc-heap.pb.gz`.

The chart does not add these paths to Higress. For an incident, port-forward the internal Service
and read the existing internal token without printing it into logs:

```bash
kubectl -n <namespace> port-forward service/<release>-internal 18081:8081
INTERNAL_TOKEN="$(kubectl -n <namespace> get secret <internal-auth-secret> \
  -o jsonpath='{.data.token}' | base64 -d)"
curl --fail --header "Authorization: Bearer ${INTERNAL_TOKEN}" \
  'http://127.0.0.1:18081/debug/pprof/profile?seconds=10' -o mwc-cpu.pb
curl --fail --header "Authorization: Bearer ${INTERNAL_TOKEN}" \
  'http://127.0.0.1:18081/debug/pprof/heap' -o mwc-heap.pb.gz
```

Inspect either capture with the standard `pprof` tool. CPU capture adds a 99 Hz sampling signal for
the requested interval. Heap sampling adds allocator bookkeeping while the diagnostics flag is
enabled; the default deployment keeps it disabled. Captures contain function names and allocation
stacks, so treat them as operationally sensitive and retain them only for the incident window.

## Loki

MWC writes structured operational logs to stdout/stderr and never logs injection plaintext. The
existing Promtail/Loki installation can retain these bounded labels:

- `mwc_installation`
- `mwc_organization_id`
- `mwc_owner_user_id`
- `mwc_workspace_id`

Use Loki retention and tenant isolation appropriate to the deployment. MWC does not embed or
operate another log store.

## Reused cluster infrastructure

ServiceMonitor and PrometheusRule reuse Prometheus Operator. Dashboards reuse Grafana. Home PVC
capacity and snapshots reuse kubelet metrics and Longhorn. Logs reuse Promtail and Loki. Alert
routing reuses Alertmanager. No part of the health, metrics or diagnostics implementation depends
on Tailscale, Coder or Coder Premium.

For large public installations, use PostgreSQL and multiple identical control-plane replicas.
SQLite remains single-replica. Keep BuildKit disabled in templates that do not build images, and do
not share a workspace Pod between mutually untrusted users merely to reduce cost.
