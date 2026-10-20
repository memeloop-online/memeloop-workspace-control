# Memeloop Workspace Control Helm chart

This chart installs the MWC control plane, workspace reconciler, optional SSH
jump host, access routes, and monitoring resources.

## Requirements

- Kubernetes with a default StorageClass or an explicitly selected class
- the `memeloop-workspace-control` release namespace
- an immutable installation ID
- existing Secrets for the envelope-encryption key and internal-auth token
- a persistent OpenSSH host-key Secret when the jump host is enabled
- pinned MWC, ttyd, BuildKit, and jump-host images for reproducible deployments

The chart supports two database modes:

- `sqlite`: one StatefulSet replica with one RWO control-plane PVC
- `postgresql`: a Deployment backed by an external PostgreSQL database, with
  optional horizontal autoscaling

## Install

Create the namespace and required Secrets through your preferred secret
management workflow, then prepare a values file:

```yaml
installationId: example
mode: sqlite

image:
  repository: ghcr.io/memeloop-online/memeloop-workspace-control
  digest: sha256:<verified-digest>

secrets:
  encryptionSecretName: mwc-encryption
  internalAuthSecretName: mwc-internal-auth

workspace:
  storageClassName: longhorn
  ttydImage: ghcr.io/example/ttyd@sha256:<verified-digest>
  buildkitImage: moby/buildkit@sha256:<verified-digest>

jumpHost:
  enabled: false
```

Install or update the release:

```bash
helm upgrade --install memeloop-workspace-control \
  deploy/helm/memeloop-workspace-control \
  --namespace memeloop-workspace-control \
  --create-namespace \
  --values values.production.yaml
```

Use image digests in production. A configured digest takes precedence over its
tag.

## Storage

`workspace.storageClassName` stores durable workspace home volumes.
`workspace.scratchStorageClassName` defaults to `mwc-local-scratch` and selects a CSI class for
regenerable build and cache data, separate from Home.

SQLite uses the `sqlite.size` and `sqlite.storageClassName` settings. Set
`sqlite.existingClaim` on a fresh installation to mount a pre-created claim;
it cannot be combined with `sqlite.storageClassName`.

### Storage shrink

Build and review the dedicated helper in `images/storage-shrink` in image CI.
Configure `workspace.storageShrink.helperImage` with its exact `@sha256:` image
reference and explicitly set `workspace.storageShrink.xattrPolicy: posix-user`.
There is no workspace-image fallback and no runtime package installation.
Leaving these unset makes the shrink creation endpoint return `503`, without
stopping workspaces. The chart validates digest/policy and conditionally grants
PV `get/patch/delete`, StorageClass `get`, and Longhorn Volume `get` to the control
plane. The helper receives none of these permissions or a service-account token.

`workspace.storageShrink.longhornNamespace` defaults to `longhorn-system`.
Home and scratch must use filesystem CSI volumes; live scratch is retained before
Pod scale-down. Read [the product guide](../../../docs/storage-shrink.md) for
one-click `reclaim_source`, retained-backup confirmation and xattr limitations.

## Access

Access features appear in the console when their values and template settings
are present:

| Feature | Main values |
| --- | --- |
| Internal SSH | `workspace.internalSshHost` |
| Public SSH | `public.sshHost`, `jumpHost.*`, `higress.*` |
| Web terminal | `public.webShellDomain`, `public.webShellOrigin`, `workspace.ttydMtls.*`, `higress.ttydMtls.*` |
| Port mappings | `public.portMappingDomain`, `higress.extAuthPluginUrl` |

Public routes use the configured Higress Gateway. Web terminal and port
mapping domains require matching DNS and TLS configuration. Port mappings use
ClusterIP Services and authenticated HTTPS URLs.

## Workspace networking

Templates can select the `internet_only` egress policy. Configure
`workspace.egress.dnsNamespace` and `workspace.egress.dnsPodLabels` to match
the cluster DNS Pods. Add cluster- or provider-specific address ranges to
`workspace.egress.additionalBlockedCidrs` and public hostnames to
`workspace.egress.dynamicBlockedHosts`.

Keep `networkPolicy.enabled` enabled for installations that host untrusted
workloads, and validate policy enforcement with the installed CNI.

## Monitoring

The internal Service exposes `/metrics`. The chart can create:

- a `ServiceMonitor` with `monitoring.serviceMonitor.enabled`
- storage and queue alerts with `monitoring.prometheusRule.enabled`
- PostgreSQL HPA metrics with `autoscaling.customMetrics.enabled`

Set `monitoring.prometheusUrl` when the console should display PVC usage.
Runtime diagnostics are opt-in through `monitoring.diagnostics.enabled` and
remain on the authenticated internal listener.

## Plugins

Administrators can install and configure plugin packages through the product.
For operator-managed startup packages, set one of
`plugins.existingConfigMap` or `plugins.existingClaim`. The two sources are
mutually exclusive.

## Example values

- `values.example.yaml`: PostgreSQL installation with public access settings

Review `values.yaml` for the complete value reference.
