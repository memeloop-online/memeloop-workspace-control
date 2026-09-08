# Web Shell upstream mTLS

This optional configuration makes the Higress-to-ttyd upstream mutually authenticated.
It protects ttyd when a CNI or cross-node SNAT makes an otherwise permitted gateway source
address reachable directly. It does not replace external-auth: browser WebSocket requests
still require and atomically consume the existing one-time Web Shell ticket.

Do not enable this until the certificate material and the Higress controller behavior have
been verified in the target cluster. Production acceptance is not yet complete. This feature
does not widen any NetworkPolicy source, CIDR, Service type, NodePort, or host-port access.

The initial 2026-09-08 canary failed server-name validation on Higress 2.2.3: a wrong SNI still
received HTTP 200. Ingress annotations alone are insufficient to assert certificate SAN
verification. A subsequent canary using an exact-service EnvoyFilter passed the certificate
tests below. Its integration into the product lifecycle is in progress; production enablement
remains gated on that integration and authenticated WebSocket acceptance.

## Enablement

Set all three environment variables in the control-plane process, or set none of them:

```text
MWC_TTYD_MTLS_SERVER_SECRET=ttyd-server-tls
MWC_HIGRESS_MTLS_CLIENT_SECRET_NAMESPACE=higress-system
MWC_HIGRESS_MTLS_CLIENT_SECRET_NAME=ttyd-client
```

Partial configuration is rejected at startup. Values must be valid Kubernetes Secret names;
the Higress Secret namespace is a DNS label, not a dotted Namespace path.
The client Secret Namespace must equal `MWC_HIGRESS_NAMESPACE` (default `higress-system`);
the SAN filter and its scoped permissions target that gateway Namespace.

The Helm chart supplies the same values and injects the environment variables:

```yaml
workspace:
  ttydMtls:
    serverTlsSecretName: ttyd-server-tls
higress:
  ttydMtls:
    clientSecretNamespace: higress-system
    clientSecretName: ttyd-client
```

The chart rejects any partial set. The server Secret is resolved in the shared
`memeloop-workspace-control` Namespace, where workspace Pods run. The Higress client Secret
is referenced by Higress in its configured Namespace; it is not copied into a workspace.

## Required Secrets and certificate roles

`ttyd-server-tls` must contain exactly the required mounted keys below. Kubernetes refuses
the Pod if a required key is absent; it is deliberately not an optional volume.

```text
tls.crt  ttyd server certificate (EKU: serverAuth)
tls.key  ttyd server private key
ca.crt   CA certificate that validates the Higress client certificate
```

The volume is mounted read-only only at `/etc/mwc-ttyd-tls` in the `ttyd` container. The
workspace container does not mount it. ttyd is started with `--ssl`, `--ssl-cert`,
`--ssl-key`, and `--ssl-ca`, so it requires a valid client certificate.

The Higress client Secret is an operator-managed TLS client credential, conventionally with
the standard `tls.crt` and `tls.key` keys. Higress also requires its companion CA Secret named
`<clientSecretName>-cacert`, with the documented key `cacert`; that CA validates ttyd's server certificate. The client Secret's
private key must remain available only to Higress, never to a workspace Pod, ConfigMap, API
response, or browser.

Use separate certificate roles and preferably separate issuing CAs:

- ttyd's certificate has `serverAuth`; Higress validates it using the companion `-cacert` CA.
- Higress's certificate has `clientAuth`; ttyd validates it using `ttyd-server-tls/ca.crt`.
- A ttyd server private key is neither a client credential nor a CA signing key. Do not reuse
  it as the Higress client key or let it sign client certificates. Keep CA private keys outside
  workload Secrets.

Operators may provision the certificates through their existing cert-manager policy or normal
offline OpenSSL process. This component does not create a CA, issue certificates, or distribute
CA private keys.

## Name and SNI requirements

The current Ingress requests HTTPS upstream, certificate verification, a client Secret reference,
and SNI equal to the target Service FQDN. An explicit SAN-validation configuration is still required
for the installed Higress version:

```text
w-<workspace-short-id>.memeloop-workspace-control.svc.cluster.local
```

The ttyd server certificate must cover that SNI. A wildcard
`*.memeloop-workspace-control.svc.cluster.local` covers exactly one leftmost label and is
appropriate for `w-<workspace-short-id>` service names. It does not cover deeper names,
another Namespace, or arbitrary `w-*` text outside that single label. Alternatively issue SANs
for the concrete Service names.

The validated gateway configuration adds a standard EnvoyFilter with `context: GATEWAY`,
the exact Service FQDN and port `7681`. Its TLS combined validation context retains the SDS
CA reference and adds a DNS SAN matcher for that FQDN. The existing client-certificate SDS
entry is preserved, not appended a second time. This filter belongs in the gateway Namespace;
a filter in the workspace Namespace does not configure a gateway in another Namespace.
Applying the Kubernetes object is not evidence that the gateway has acknowledged it.

The existing `nginx` IngressClass is retained because the installed Higress controller maps
that class in this deployment. Do not change the class merely to enable mTLS.

## Planned disable sequence

The lifecycle implementation uses two stages so installations that never enabled this feature
do not need an EnvoyFilter CRD or gateway permissions. Complete these checks before removing
the mTLS configuration or its namespace-scoped Role:

1. Keep all mTLS settings and gateway permissions. Set both `public.webShellDomain` and
   `public.webShellOrigin` to empty, and wait until all control-plane replicas run that
   configuration. This intentionally disables Web Shell access.
2. Remove the owned Web Shell Ingresses first and confirm they are absent; only then remove
   their exact per-workspace SAN filters. Match the database workspace identity and installation
   ownership labels for every deletion. Preserve unrelated HTTP port-mapping Ingresses and
   gateway filters. Stop on an ownership mismatch.
3. Confirm no owned Web Shell Ingress or SAN filter remains, then remove all three mTLS settings
   and the conditional gateway Role/RoleBinding.

Do not assume a control-plane restart automatically requeues every settled workspace. A completed
job is not a continuous resync. If no reconcile is queued for an existing workspace, the operator
must perform the scoped cleanup in step 2; do not start a stopped workspace merely to trigger it.
Changing the gateway Namespace likewise requires cleanup in the old Namespace before removing
its permissions. These steps do not stop workspace SSH or delete user volumes.

## Rotation and operational checks

Rotate server and client material with the operator's established certificate workflow. A
Secret update alone is not a complete ttyd rotation procedure: restart the affected ttyd Pods
so their TLS context is recreated. This interrupts existing Web Shell connections; schedule and
communicate the impact. Coordinate client and server trust overlap before removing an old CA.

Before production acceptance, validate in the real cluster that:

- Higress can establish an HTTPS upstream connection with the configured client certificate and
  verifies the Service-FQDN SNI.
- A direct ClusterIP attempt without the Higress client certificate fails the ttyd TLS handshake.
- The workspace container cannot read `/etc/mwc-ttyd-tls` or either private key.
- A normal browser flow still receives external-auth and its WebSocket ticket is consumed once.
- Existing NetworkPolicy sources remain unchanged.

This is a defense for the ttyd upstream. It does not claim to protect host networking, close a
Pod startup window, or make an unverified production deployment safe.

## Verified canary — 2026-09-08

The dedicated test used Higress 2.2.3 and the released ttyd image
`sha256:6f430bf2941bbb0e2110a5211a4884018efd72ec52a217b6db1b442d5b34eed7`
from successful CI run `34254345334` (source `32945cc`). The test inspected the same gateway
Pod that handled HTTP requests and confirmed both SDS references plus the exact DNS SAN matcher.

| Case | Observed outcome |
| --- | --- |
| Correct CA and one-label wildcard server certificate | HTTP 200 |
| Server certificate issued by a different CA | HTTP 503 |
| Restore correct server certificate | HTTP 200 |
| Correct issuing CA, wrong server DNS SAN | HTTP 503 |
| Restore correct server certificate again | HTTP 200 |

Each negative case replaced only the disposable ttyd Pod with an actually different server
certificate; changing SNI alone was not used as a substitute. The terminal command was
`/bin/false`, so the temporary route did not offer an interactive shell.

GitOps test commits: `327418d`, corrected masked-CDS parser and pinned gateway `3a329a3`.
Local result: `/tmp/mwc-higress-ttyd-san-20260908.json`, process exit 0. Cleanup reported no
errors, and absence of both the unique test Namespace and gateway EnvoyFilter was independently
checked. Earlier native-ttyd tests separately rejected absent and untrusted client certificates.

This proves the tested gateway TLS configuration, not the full external sandbox. Product
resource reconciliation/deletion, certificate renewal, browser WebSocket ticket handling and
NetworkPolicy/startup/rescheduling coverage still require their own acceptance.
