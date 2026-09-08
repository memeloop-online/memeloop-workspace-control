# Web Shell upstream mTLS

This optional configuration makes the Higress-to-ttyd upstream mutually authenticated.
It protects ttyd when a CNI or cross-node SNAT makes an otherwise permitted gateway source
address reachable directly. It does not replace external-auth: browser WebSocket requests
still require and atomically consume the existing one-time Web Shell ticket.

Do not enable this until the certificate material and the Higress controller behavior have
been verified in the target cluster. Production acceptance is not yet complete. This feature
does not widen any NetworkPolicy source, CIDR, Service type, NodePort, or host-port access.

The 2026-09-08 canary failed server-name validation on Higress 2.2.3: a wrong SNI still
received HTTP 200. The current Ingress annotations are insufficient to assert certificate
SAN verification. Keep deployment gated until explicit upstream SAN validation is configured
and the negative test passes; sending SNI alone is not server-identity verification.

## Enablement

Set all three environment variables in the control-plane process, or set none of them:

```text
MWC_TTYD_MTLS_SERVER_SECRET=ttyd-server-tls
MWC_HIGRESS_MTLS_CLIENT_SECRET_NAMESPACE=higress-system
MWC_HIGRESS_MTLS_CLIENT_SECRET_NAME=ttyd-client
```

Partial configuration is rejected at startup. Values must be valid Kubernetes Secret names;
the Higress Secret namespace is a DNS label, not a dotted Namespace path.

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

The existing `nginx` IngressClass is retained because the installed Higress controller maps
that class in this deployment. Do not change the class merely to enable mTLS.

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
