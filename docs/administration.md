---
id: administration
title: Administration
sidebar_label: Administration
sidebar_position: 6
---

# Administration

This page covers the system- and organization-level management surface. All
operations are available in the console and through the REST API.

## Roles

MWC has three roles:

- **System administrator** — full control: images, users, organizations,
  node pools, audit, and platform settings.
- **Organization administrator** — manages members, quotas, locked injection
  items, and workspaces within their organization.
- **Member** — creates and operates their own workspaces and personal
  credentials.

## Image allowlist (Image Contract v1)

Workspace images are default-deny. A system administrator must explicitly
allow each image before any template can reference it:

- `GET /api/v1/admin/images` — list allowed images.
- `PUT /api/v1/admin/images` — allow or update an image entry.

Allow standard platform images or reviewed third-party OCI images. Removing an
image from the allowlist does not stop running workspaces but blocks new ones
from referencing it.

## Templates

Templates are the unit users create workspaces from (see
[Workspaces](./workspaces.md)). Administrators create, update, enable,
disable, and delete them:

- `GET /api/v1/templates` / `POST /api/v1/templates`
- `PUT /api/v1/templates/{template_id}` / `DELETE /api/v1/templates/{template_id}`
- `PUT /api/v1/templates/{template_id}/enabled`

## Users and organizations

- Users: `GET /api/v1/admin/users`, `POST /api/v1/admin/users`,
  `PUT /api/v1/admin/users/{user_id}`. Administrators can also list and revoke
  a user's API keys.
- Organizations: `GET /api/v1/organizations`, `POST /api/v1/organizations`,
  `PUT`/`DELETE /api/v1/organizations/{organization_id}`.
- Members: `GET /api/v1/organizations/{organization_id}/members`,
  `PUT`/`DELETE .../members/{user_id}`.
- Usage: `GET /api/v1/organizations/{organization_id}/usage-summary`.

List endpoints use cursor pagination (`limit`, `cursor`, `search`) and return
`items` plus an optional `next_cursor`.

## Quotas

Quotas cap resource consumption at both levels:

- Organization: `GET`/`PUT /api/v1/organizations/{organization_id}/quota`.
- User: `GET`/`PUT /api/v1/admin/users/{user_id}/quota`.

## Node pools and scaling

- `GET`/`PUT`/`DELETE /api/v1/admin/node-pools[/{name}]` — manage node pools
  that templates and placements reference.
- `GET /api/v1/admin/scaling` — capacity and scaling overview.

## Audit and observability

- `GET /api/v1/audit` — administrative audit log of privileged actions.
- `GET /api/v1/events` — SSE stream of workspace and platform events.
- `GET /livez` / `GET /readyz` — probe endpoints on both listeners.
- `GET /metrics` — Prometheus metrics on the internal listener only.

## Webhooks

Signed outbound webhooks notify external systems of platform events:

- `GET /api/v1/webhooks` / `POST /api/v1/webhooks` — list and create
  subscriptions.

Deliveries are signed so receivers can verify authenticity.
