# ttyd upstream mTLS certificate lifecycle

This directory prepares only the native cert-manager leaf certificates. It is
not an enablement action: it does not set Helm mTLS values, create an Ingress,
restart a workspace, alter Higress, or copy certificate material.

## Minimal topology

Two independently operated CA Secrets exist outside Git:

| CA Secret | Namespace | Signs | Private-key boundary |
| --- | --- | --- | --- |
| `ttyd-server-ca` | `memeloop-workspace-control` | `ttyd-server-tls` | Never leaves its operator-controlled Secret. |
| `ttyd-client-ca` | `higress-system` | `ttyd-client` | Never leaves its operator-controlled Secret. |

`certificates.yaml` creates namespaced `Issuer` objects that refer to those
pre-existing CA Secrets and 90-day ECDSA leaf `Certificate` resources. The
leaves renew 30 days early with `rotationPolicy: Always`:

- `ttyd-server-tls`: `serverAuth`, with the only DNS SAN
  `*.memeloop-workspace-control.svc.cluster.local`.
- `ttyd-client`: `clientAuth`, consumed by Higress through its established
  client-certificate SDS reference.

No CA `Certificate`, self-signed root, Secret stub, copier, CronJob, or new
controller belongs in this GitOps directory. In particular, cert-manager's
leaf `ca.crt` is the *issuing leaf CA*, while ttyd must trust the independent
client CA. Co-owning that key in `ttyd-server-tls` would make renewal unsafe.

## Required product contract before this can be enabled

The previously deployed mTLS configuration mounts one Secret with `tls.crt`, `tls.key`, and `ca.crt`.
That cannot safely be cert-manager-owned when the trust CA and issuing CA are
different. Product commit `815ff90` implements the required separate,
read-only client-CA Secret volume. Its all-or-none configuration adds
`MWC_TTYD_MTLS_CLIENT_CA_SECRET=ttyd-client-trust` (Helm:
`workspace.ttydMtls.clientCaSecretName`):

| Consumer | Credential (cert-manager managed) | Separate public trust input |
| --- | --- | --- |
| ttyd | `memeloop-workspace-control/ttyd-server-tls`, keys `tls.crt`, `tls.key` | `memeloop-workspace-control/ttyd-client-trust` Secret, key `ca.crt`, mounted only into ttyd and passed to `--ssl-ca` |
| Higress | `higress-system/ttyd-client`, keys `tls.crt`, `tls.key` | `higress-system/ttyd-client-cacert` Secret, key `cacert`, as Higress's documented `<clientSecretName>-cacert` companion |

The two trust destination Secrets contain public CA certificates only. They are not
rendered into Git. At the approved first rollout an operator copies only the
public certificate from each CA Secret into the opposite consumer destination,
without printing it, copying a CA private key, or using a broad
cross-namespace controller. Subsequent leaf renewals need no trust copy;
cert-manager updates the two leaf Secrets itself. This is intentionally manual
only for CA lifecycle, whose normal cadence is years rather than days.

The GitOps Application that adds this directory must be authorized for
`memeloop-workspace-control`, `higress-system`, and the monitoring object in `cert-manager`.
Do not apply it until both
CA Secrets have been provisioned and verified by the certificate operator.

### Initial provisioning

From the product repository, inspect the explicit target context, then run:

```bash
node --experimental-strip-types scripts/provision-ttyd-ca.ts --context default
node --experimental-strip-types scripts/provision-ttyd-ca.ts --context default --apply
```

Replace `default` with the approved context for that installation. The first command checks
destinations only. The second generates two independent ten-year ECDSA roots and creates
the four CA/trust Secrets above. It refuses existing destinations and uses Kubernetes create,
so it cannot overwrite a concurrent operator's certificate. Private temporary files are removed
on exit; signing keys remain only in the CA Secrets. Protect those Secrets in cluster backups.
On partial failure, preserve the created Secrets and investigate; this is not a rotation command.

On 2026-09-09 this bootstrap completed in the target cluster. Public-only trust contents were
compared in memory with their opposite roots; all checks passed without certificate output.
No leaf consumer or workload was changed by provisioning.

## CA lifecycle and monitoring

The CA Secrets are stable operational assets, not implicitly auto-rotated
cert-manager targets. Expiry or an unexpected CA key change is an incident,
not a leaf renewal.

**Current cluster state:** cert-manager v1.21.0 exposes its metrics Service on
`cert-manager/cert-manager:9402`, but it had no ServiceMonitor or
cert-manager PrometheusRule before this prepared manifest. The installed
Prometheus selects `release: monitoring` ServiceMonitors/Rules from all
Namespaces. `monitoring.yaml` therefore adds a ServiceMonitor matching the
live controller Service labels and its `http-metrics` port. It sets
`honorLabels: true` so cert-manager's own Certificate `namespace` label stays
available to rules, which are limited to these two leaf Certificate names.
This remains unobserved until the reviewed GitOps sync is complete; do not
describe it as active beforehand.

The live v1.21.0 metrics endpoint confirms the exact metric labels are `name`
and `namespace` (not `exported_namespace`). The prepared leaf alerts are:
missing readiness telemetry or not Ready for 10 minutes (critical), past cert-manager's renewal timestamp for
15 minutes (critical), and expiry within 14 days (warning). They use
`certmanager_certificate_ready_status`,
`certmanager_certificate_renewal_timestamp_seconds`, and
`certmanager_certificate_expiration_timestamp_seconds` respectively.

```promql
certmanager_certificate_ready_status{condition="True",name=~"ttyd-(server|client)-certificate",namespace=~"memeloop-workspace-control|higress-system"} != 1
```

**Stable-CA expiry:** a CA Secret is not a cert-manager Certificate and has no
cert-manager expiry metric. No exporter, CronJob, or periodic synchronization
component is proposed or included here. Until the existing monitoring
foundation is separately assessed for a least-privilege CA-expiry probe, the
certificate operator must record the two CA NotAfter dates in the approved
operations inventory and review them at least monthly. This is a manual
control, not an alert.

For a planned CA rollover:

1. Create the replacement CA outside Git and retain the old CA.
2. Put an old+new public bundle in the opposite consumer trust destination.
3. Issue/reissue the affected leaf from the new CA, confirm its EKU/SAN and
   both the gateway SDS and ttyd trust paths.
4. Recreate affected ttyd Pods in the approved maintenance window; a mounted
   Secret update alone does not recreate ttyd's TLS context.
5. After all old leaves have expired and negative/positive mTLS tests pass,
   remove the old public CA in a later reviewed change.

Never store, log, print, commit, or distribute `tls.key`, a CA private key, or
the complete Secret object. Before setting the product's all-or-none mTLS
configuration, run the production checks in `docs/WEB-SHELL-MTLS.md`: exact
Higress SAN validation, valid and invalid client/server trust paths, direct
ClusterIP rejection, and the authenticated browser WebSocket flow.
