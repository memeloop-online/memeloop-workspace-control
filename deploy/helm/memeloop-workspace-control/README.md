# Helm deployment

The chart supports exactly two runtime shapes:

- `mode=sqlite`: one StatefulSet replica and one RWO PVC.
- `mode=postgresql`: a Deployment plus an optional HPA. PostgreSQL is external
  and its URL comes from a Kubernetes Secret.

The service exposes Prometheus text at `/metrics`. After a metrics adapter maps
`rate(mwc_http_requests_total)` to `mwc_http_requests_per_second` and publishes
`mwc_jobs_pending`, PostgreSQL installations can enable `autoscaling.customMetrics` so the HPA
uses request rate and task backlog in addition to CPU and memory.
CPU and memory requests are set by default because utilization-based HPA metrics have no valid
denominator without them. The chart rejects a PostgreSQL autoscaling deployment if either request
is removed.

Every install requires an immutable `installationId`, a 32-byte envelope key,
an independent internal-auth token, a pinned ttyd image, and a persistent
OpenSSH host-key Secret. The chart never generates or stores those values in a
rendered manifest.

Higress prerequisites are intentionally explicit: Gateway API CRDs must be
installed, the referenced Higress Gateway must allow routes from this namespace,
and it must expose a TCP listener and Service port 22. The chart creates one
fixed TCPRoute from that listener to the standard OpenSSH jump Deployment; it
does not allocate a port per workspace. Set `higress.extAuthPluginUrl` to the
pinned official Higress ext-auth plugin OCI URL to protect all `/shell/` paths.
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
