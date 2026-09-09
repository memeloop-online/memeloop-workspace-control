# ttyd with the stock OpenSSH client and enforceable mTLS

This image builds unmodified upstream ttyd source at
`40e79c706be14029b391f369bee6613c31667abb` (the 1.7.7 release commit), rather
than copying the upstream Linux binary. It builds libwebsockets 4.3.3 against
OpenSSL with Mbed TLS disabled, so ttyd's `--ssl-ca` client-certificate
requirement is enforceable.

The image adds only the standard OpenSSH client required for ttyd to connect to
the workspace container over localhost. It does not implement a terminal or SSH
protocol. It continues to run as `root` and exposes ttyd at `/usr/bin/ttyd`.

The Dockerfile verifies the upstream source archives before building:

| Component | Upstream revision | SHA-256 archive checksum |
| --- | --- | --- |
| ttyd | `40e79c706be14029b391f369bee6613c31667abb` | `1f6f53a5021a2cee4af9b975f3dc7a3185bd261c23a84f7b680285f6c1a50341` |
| libwebsockets | `4415e84c095857629863804e941b9e1c2e9347ef` (`v4.3.3`) | `a85b3dbcf42b97722f829538acffca50d77441cec5e76b8223ac36611ae10332` |

After building a local image, run the isolated Docker-capable test:

```bash
bash tests/ttyd_mtls.sh mwc-ttyd-ci
```

It creates temporary test-only CA, server, valid-client, and wrong-CA-client
certificates. It verifies valid-client access using default TLS and forced TLS
1.2 on both `localhost` (SNI) and `127.0.0.1` (no SNI), then verifies absent
and wrong-CA client certificates fail on both paths while curl still validates
the matching server CA.
