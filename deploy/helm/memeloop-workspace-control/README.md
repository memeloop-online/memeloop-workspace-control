# Helm deployment

The chart supports exactly two runtime shapes:

- `mode=sqlite`: one StatefulSet replica and one RWO PVC.
- `mode=postgresql`: a Deployment plus an optional HPA. PostgreSQL is external
  and its URL comes from a Kubernetes Secret.

The service exposes Prometheus text at `/metrics`. After a metrics adapter maps
`rate(mwc_http_requests_total)` to `mwc_http_requests_per_second` and publishes
`mwc_jobs_pending`, PostgreSQL installations can enable `autoscaling.customMetrics` so the HPA
uses request rate and task backlog in addition to CPU and memory.

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

For reproducible deployments set `image.digest` and `jumpHost.image.digest` to
the verified `sha256:...` values published by CI. A digest takes precedence over
the corresponding tag, and the chart rejects malformed digest values.

Example values are in `values.example.yaml`. Install each coexisting instance
into its own namespace and use a separate database, secrets, ServiceAccount,
PVC, domains, and (when public SSH is enabled) LoadBalancer IP or shared jump
facility.
