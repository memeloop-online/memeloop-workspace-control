---
id: intro
title: Documentation
sidebar_label: Overview
sidebar_position: 1
slug: /
---

# Memeloop Workspace Control documentation

Memeloop Workspace Control (MWC) is a Kubernetes workspace control plane. It
provisions isolated, per-user development workspaces from reviewed templates,
injects credentials and files at creation time, and exposes each workspace over
SSH, a browser terminal, and authenticated HTTP port mappings.

This documentation is organized by audience:

- **Users** — create and connect to workspaces, manage personal credentials and
  API keys: start with [Quick start](./quickstart.md), then
  [Workspaces](./workspaces.md), [Credentials and files](./credentials-and-files.md),
  and [Access: SSH, web terminal, port mappings](./access.md).
- **Administrators** — manage images, templates, users, organizations, quotas,
  and node pools: see [Administration](./administration.md).
- **API consumers** — authenticate with scoped API keys and integrate through
  the REST API: see [API reference](./api.md).
- **Plugin developers** — build WebAssembly policy and UI plugins against the
  WIT interface: see [Plugin development](./plugin-development.md).

Cross-cutting topics:

- [Security model](./security-model.md) — isolation, authentication, and data
  protection guarantees.
- [FAQ](./faq.md) — common operational and usage questions.

The authoritative machine-readable API contract is served by each deployment at
`GET /api/v1/openapi.json`.

## License

Memeloop Workspace Control is available under the
[Apache License 2.0](https://github.com/memeloop-online/memeloop-workspace-control/blob/main/LICENSE).
Attribution details are listed in the repository [NOTICE](https://github.com/memeloop-online/memeloop-workspace-control/blob/main/NOTICE).
