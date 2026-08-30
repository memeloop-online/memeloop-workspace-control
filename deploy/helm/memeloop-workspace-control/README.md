# Helm deployment

The chart supports exactly two runtime shapes:

- `mode=sqlite`: one StatefulSet replica and one RWO PVC.
- `mode=postgresql`: a Deployment plus an optional HPA. PostgreSQL is external
  and its URL comes from a Kubernetes Secret.

The service exposes Prometheus text at `/metrics`, including platform and per-user
workspace/resource aggregates. Set `monitoring.serviceMonitor.enabled=true` when
the Prometheus Operator is installed. After a metrics adapter maps
`rate(mwc_http_requests_total)` to `mwc_http_requests_per_second` and publishes
`mwc_jobs_pending`, PostgreSQL installations can enable `autoscaling.customMetrics` so the HPA
uses request rate and task backlog in addition to CPU and memory.
CPU and memory requests are set by default because utilization-based HPA metrics have no valid
denominator without them. The chart rejects a PostgreSQL autoscaling deployment if either request
is removed.

Set `monitoring.prometheusUrl` to an in-cluster Prometheus base URL to show PVC usage,
capacity, and available bytes. The URL is optional; the control plane uses bounded,
fixed queries and needs neither Kubernetes node proxy nor Pod exec permissions.

Every install requires an immutable `installationId`, a 32-byte envelope key,
an independent internal-auth token, a pinned ttyd image, and a persistent
OpenSSH host-key Secret. The chart never generates or stores those values in a
rendered manifest.

Higress prerequisites are intentionally explicit. Gateway API CRDs and a referenced Gateway are
needed only for the fixed public API HTTPRoute and public SSH TCPRoute; the Gateway must allow
routes from this namespace and public SSH additionally needs listener and Service port 22. Web
Shell instead uses a built-in `networking.k8s.io/v1` Ingress in each workspace Namespace, with
`ingressClassName: nginx`, and therefore needs neither Gateway API CRDs nor ReferenceGrant. Set
`higress.extAuthPluginUrl` to the pinned official Higress ext-auth plugin OCI URL to protect all
`/shell/` paths. The example pins the official ext-auth 1.0.0 artifact by digest; mirror that OCI
artifact into Harbor only if gateway nodes cannot reach the official registry, then update the
value to the verified mirror digest.
The chart refuses to render a Web Shell domain without that plugin and an exact
`https://<webShellDomain>` public origin.
Set `higress.podLabels` to the labels actually present on the K3S Higress gateway Pods. The same
selector is used by both the control-plane and workspace NetworkPolicies.

For reproducible deployments set `image.digest` and `jumpHost.image.digest` to
the verified `sha256:...` values published by CI. A digest takes precedence over
the corresponding tag, and the chart rejects malformed digest values.

Public PostgreSQL example values are in `values.example.yaml`; the internal SQLite shape is in
`values.internal.example.yaml`. Install each coexisting instance
into its own namespace and use a separate database, secrets, ServiceAccount,
PVC, domains, and (when public SSH is enabled) LoadBalancer IP or shared jump
facility.

Before installing on K3S, run `scripts/k3s/preflight.sh`. After rollout, run
`scripts/k3s/verify-installation.sh`; both scripts are read-only and require explicit environment
variables so they cannot silently target a default installation.

## WASM plugins

Plugin packages may be supplied through the administrator-only inspection and confirmation API;
their bytes, assets, approvals, enabled state, and optimistic version are persisted in the
authoritative database and hot-reloaded. An optional operator-controlled startup source can also be
mounted by setting exactly one of `plugins.existingConfigMap` or `plugins.existingClaim`; changes to
that read-only startup mount require a Pod restart. ConfigMap users must map each package below its
own first-level directory with `plugins.configMapItems`.

Mounting a startup package is its GitOps approval. Any visible malformed,
incompatible, duplicated, symlinked, escaping, or oversized package makes startup fail closed.
Runtime policy traps and limits reject only that workspace creation; existing workspaces and cleanup
actions remain available.
