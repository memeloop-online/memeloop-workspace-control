# Helm deployment

The chart supports exactly two runtime shapes:

- `mode=sqlite`: one StatefulSet replica and one RWO PVC.
- `mode=postgresql`: a Deployment plus an optional HPA. PostgreSQL is external
  and its URL comes from a Kubernetes Secret.

The service exposes OpenMetrics at `/metrics`, including HTTP latency/errors, active streams,
upstream calls, durable queues, process/allocator memory, plugin state, and platform/per-user
workspace aggregates. Set `monitoring.serviceMonitor.enabled=true` when the Prometheus Operator is
installed; it scrapes the existing internal Service and the chart permits its configured monitoring
namespace through NetworkPolicy. After a metrics adapter maps
`rate(mwc_http_requests_total)` to `mwc_http_requests_per_second` and publishes
`mwc_jobs_pending`, PostgreSQL installations can enable `autoscaling.customMetrics` so the HPA
uses request rate and task backlog in addition to CPU and memory.
CPU and memory requests are set by default because utilization-based HPA metrics have no valid
denominator without them. The chart rejects a PostgreSQL autoscaling deployment if either request
is removed.

Set `monitoring.diagnostics.enabled=true` only during an incident to activate release CPU and
jemalloc heap pprof capture on the existing internal listener. These endpoints require the internal
Bearer token and are never added to Higress. See `docs/OBSERVABILITY.md` for endpoint contracts,
overhead, port-forward commands and retention guidance.

Set `monitoring.prometheusRule.enabled=true` to install the storage and queue alerts. The rule
covers Home PVC and node ephemeral-storage request bands at 80% (warning) and 90% (critical),
failed jobs after 10 minutes, and a pending-job age above 15 minutes. The rule also records the
workspace Home usage and node ephemeral-storage request percentages for Grafana. Alert duration
for the storage bands follows `monitoring.prometheusRule.warningFor` and
`monitoring.prometheusRule.criticalFor`.

Set `monitoring.prometheusUrl` to an in-cluster Prometheus base URL to show PVC usage,
capacity, and available bytes. The URL is optional; the control plane uses bounded,
fixed queries and needs neither Kubernetes node proxy nor Pod exec permissions.

The API-key and profile endpoints are available at `/api/v1/me/api-keys` and
`/api/v1/me/profile`. New API keys carry explicit scopes and an expiry within 365 days; the
plaintext token is returned once at creation. Profile avatars are local PNG, JPEG, or WebP
uploads capped at 512 KiB. Workspace, organization, user, and membership list endpoints use
`limit`, `cursor`, and `search` parameters and return `items` with an optional `next_cursor`, so
administrative screens can load large installations incrementally.

Every install requires an immutable `installationId`, a 32-byte envelope key,
an independent internal-auth token, a pinned ttyd image, and a persistent
OpenSSH host-key Secret. The chart never generates or stores those values in a
rendered manifest.

Higress prerequisites are intentionally explicit. Gateway API CRDs and a referenced Gateway are
needed only for the fixed public API HTTPRoute and public SSH TCPRoute; the Gateway must allow
routes from this namespace and public SSH additionally needs listener and Service port 22. Web
Shell instead uses a built-in `networking.k8s.io/v1` Ingress in each workspace Namespace, with
`ingressClassName: nginx`, and therefore needs neither Gateway API CRDs nor ReferenceGrant. Set
`higress.extAuthPluginUrl` to the pinned official Higress ext-auth plugin OCI URL. When either
public Web Shell or HTTP port mappings is enabled, the chart creates one `<installation>-access-auth`
WasmPlugin. Its ordered match rules put the exact Web Shell host first and the full-label port
mapping wildcard second; each rule keeps its own fail-closed policy, inner blacklist, authorization
endpoint, and response-header policy. The example pins the official ext-auth 1.0.0 artifact by
digest; mirror that OCI artifact into Harbor only if gateway nodes cannot reach the official
registry, then update the value to the verified mirror digest.
The chart refuses to render a Web Shell domain without that plugin and an exact
`https://<webShellDomain>` public origin.
Set `higress.podLabels` to the labels actually present on the K3S Higress gateway Pods. The same
selector is used by both the control-plane and workspace NetworkPolicies.

### Workspace HTTP port mappings

Set `public.portMappingDomain` to a DNS suffix dedicated to workspace applications, such as
`ports.example.com`. The deployment then uses `p-<mapping-id>.ports.example.com` hostnames. A
wildcard DNS record and matching wildcard TLS certificate for `*.ports.example.com` are required.
Configure the wildcard in Higress `credentialConfig` without an ACME issuer and enable
`fallbackForInvalidSecret`; the generated Ingress uses an intentionally absent placeholder Secret
so Higress resolves the certificate centrally instead of copying private keys into workspace
namespaces.
Higress attaches fail-closed external authentication through the shared `access-auth` plugin and
the valid full-label wildcard route match, then the port-mapping rule selects only `p-*` mapping
hosts with its inner blacklist. This two-stage match is required because Higress route matching
does not accept a partial-label wildcard such as `p-*.example.com`. The workspace application is
reached through a ClusterIP Service; NodePort and hostPort are outside this path.

The API returns a stable HTTPS URL for each mapping. The `open` action creates a one-use bootstrap
URL valid for 60 seconds. Requesting that URL reaches the mapping Ingress and invokes the internal
port-mapping authorization endpoint in the same access-auth pass. On success, the endpoint returns
`303 See Other` with `Location` and `Set-Cookie`; the browser follows the redirect and receives the
`__Host-mwc-port-session` `HttpOnly`, `Secure`, `SameSite=Lax` cookie in one exchange. Its session
lifetime is eight hours. There is no per-mapping public bootstrap backend. Deleting a mapping
revokes its tickets and sessions and queues reconciliation of the owned Ingress, Services, and
NetworkPolicy. The mapping domain is passed to the control plane as `MWC_PORT_MAPPING_PUBLIC_DOMAIN`.

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
