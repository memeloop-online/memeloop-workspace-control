# Memeloop Workspace Control

**[简体中文](README.zh-CN.md)** · **[Documentation](https://memeloop-online.github.io/memeloop-workspace-control/)**

Memeloop Workspace Control (MWC) is an open-source Kubernetes control plane
for isolated, per-user development workspaces. It delivers workspaces from
reviewed templates and an image allowlist, injects encrypted credentials and
files at startup, and provides access over SSH, a browser terminal, and
authenticated port mappings.

![Workspace resource overview](docs-site/static/img/screenshots/workspaces-desktop.png)

## Capabilities

- **Workspace lifecycle** — template-based create, start, stop, restart, and
  delete, with persistent home volumes, bounded temporary storage, and
  node-pool scheduling.
- **Credentials & files** — organization, user, and workspace scopes with
  encrypted storage and organization-locked items.
- **Workspace access** — OpenSSH, a web terminal, and authenticated HTTPS
  port mappings, enabled per deployment and template.
- **Administration** — image allowlist, templates, organizations and members,
  two-level quotas, and an audit log.
- **API-first** — scoped API keys, cursor pagination, SSE events, signed
  webhooks, and an OpenAPI contract.
- **WebAssembly plugins** — creation policies, API middleware, custom routes,
  and console surfaces through a versioned WIT interface.

## Architecture

MWC runs as a single control-plane service backed by a SQL database. It drives
the Kubernetes API to provision an isolated environment per workspace from a
reviewed template and an allowlisted image, mounting a persistent home volume
and injecting credentials and files from encrypted storage at startup. User
traffic reaches workspaces over SSH, a browser terminal, or authenticated
port-mapping proxies. Every operation flows through the REST API, which the
web console, scoped API keys, and sandboxed WebAssembly plugins all build on.

## Quick start

```bash
cargo run -- \
  --installation-id demo \
  --listen-address 127.0.0.1:8080 \
  --database-url 'sqlite://data/control-plane.sqlite?mode=rwc' \
  --instance-id local
```

For Kubernetes deployments, use the Helm chart in
`deploy/helm/memeloop-workspace-control`. Encrypted injection and webhooks
require `MWC_ENCRYPTION_KEY` and `MWC_INTERNAL_AUTH_TOKEN`, supplied from a
Secret. See the [quick-start guide](docs/quickstart.md) for details.

## Documentation

Full bilingual documentation is published at
<https://memeloop-online.github.io/memeloop-workspace-control/>.

- [Quick start](docs/quickstart.md)
- [Workspaces](docs/workspaces.md) · [Credentials and files](docs/credentials-and-files.md) · [Access](docs/access.md)
- [Administration](docs/administration.md) · [API](docs/api.md) · [Plugin development](docs/plugin-development.md)
- [Security model](docs/security-model.md) · [FAQ](docs/faq.md)

## License

This project is licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE)
and [NOTICE](NOTICE).
