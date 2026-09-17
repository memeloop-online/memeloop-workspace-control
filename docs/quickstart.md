---
id: quickstart
title: Quick start
sidebar_label: Quick start
sidebar_position: 2
---

# Quick start

This page gets a local control plane running and walks through creating a first
workspace. For production deployment on Kubernetes, use the Helm chart under
`deploy/helm/memeloop-workspace-control` in the repository.

## Run the control plane locally

MWC is a single Rust binary. Create the database directory and an initial
system administrator before starting the server:

```bash
mkdir -p data
export MWC_ADMIN_TOKEN="$(openssl rand -base64 32)"

cargo run -- \
  --installation-id demo \
  --listen-address 127.0.0.1:8080 \
  --database-url 'sqlite://data/control-plane.sqlite?mode=rwc' \
  --instance-id local \
  admin create-user --display-name Administrator --system-admin
```

`MWC_ADMIN_TOKEN` is the initial API key. Save it in your password manager.
For encrypted injections and signed webhooks, set two independent random
values before starting the server:

```bash
export MWC_ENCRYPTION_KEY="$(openssl rand -base64 32)"
export MWC_INTERNAL_AUTH_TOKEN="$(openssl rand -base64 32)"
```

Start the server in the same terminal with the same database and installation
settings:

```bash
cargo run -- \
  --installation-id demo \
  --listen-address 127.0.0.1:8080 \
  --database-url 'sqlite://data/control-plane.sqlite?mode=rwc' \
  --instance-id local
```

Open a second terminal and set the API examples to use the running server and
the saved initial key:

```bash
export BASE='http://127.0.0.1:8080'
export API_KEY='<saved MWC_ADMIN_TOKEN value>'
curl "$BASE/api/v1/me" -H "Authorization: Bearer $API_KEY"
```

The web console is served from the same listener. Health endpoints:

- `GET /livez` — process liveness.
- `GET /readyz` — database connectivity check.
- `GET /api/v1/system/info` — version and installation metadata.

Workspace provisioning also requires Kubernetes access. Run the process with a
working kubeconfig or service account and set:

```bash
export MWC_KUBERNETES_ENABLED=true
export MWC_TTYD_IMAGE='tsl0922/ttyd:1.7.7'
```

For Kubernetes deployments, inject the encryption and internal-auth values
from Secrets. The Helm chart wires these values into the control plane.

## Create your first workspace

1. **Allow an image.** Workspace images are default-deny (Image Contract v1). A
   system administrator must first allow an image through the admin console or
   `PUT /api/v1/admin/images`.
2. **Create a template.** Templates describe the image, resources, storage,
   network egress policy, and placement of a class of workspaces. Manage them
   in the console or through `POST /api/v1/templates`.
3. **Add credentials.** Define environment variables, files, or SSH public keys
   at organization, user, or workspace scope. See
   [Credentials and files](./credentials-and-files.md).
4. **Create the workspace.** Copy the organization, owner, and template IDs
   from the console or their API responses, then run:

   ```bash
   export ORG_ID='<organization-id>'
   export OWNER_ID='<user-id>'
   export TEMPLATE_ID='<template-id>'

   curl -X POST "$BASE/api/v1/workspaces" \
     -H "Authorization: Bearer $API_KEY" \
     -H "Idempotency-Key: workspace-$(date +%s)" \
     -H 'Content-Type: application/json' \
     -d "{\"organization_id\":\"$ORG_ID\",\"owner_id\":\"$OWNER_ID\",\"name\":\"my-workspace\",\"template_id\":\"$TEMPLATE_ID\"}"
   ```

5. **Connect.** Once the workspace is ready, connect over SSH, the browser
   terminal, or publish an application port. See
   [Access](./access.md).

## Next steps

- [Workspaces](./workspaces.md) — lifecycle, states, storage, placement.
- [API reference](./api.md) — API keys, scopes, pagination, events, webhooks.
- [Security model](./security-model.md) — what the platform guarantees.
