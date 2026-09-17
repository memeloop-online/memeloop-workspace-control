---
id: api
title: API reference
sidebar_label: API
sidebar_position: 7
---

# API reference

The REST API is versioned under `/api/v1`. Each deployment serves the
authoritative machine-readable contract at `GET /api/v1/openapi.json`; this
page summarizes the integration model.

## Authentication

API consumers authenticate with personal API keys:

```bash
curl "$BASE/api/v1/me" -H "Authorization: Bearer $API_KEY"
```

Keys are managed in the console or through `GET`/`POST /api/v1/me/api-keys`
and `DELETE /api/v1/me/api-keys/{key_id}`. Rules:

- Every key carries at least one fine-grained scope.
- Expiry is a Unix timestamp at most 365 days in the future.
- The plaintext key is returned only once, in the creation response.
- Keys can optionally be restricted to specific template IDs: `null` means no
  additional restriction, `[]` means the key cannot create workspaces from any
  template.

Available scopes: `create_workspace`, `read_workspace`, `connect_workspace`,
`change_workspace_state`, `delete_workspace`, `manage_organization`,
`manage_members`, `manage_locked_injections`, `manage_system`,
`manage_api_keys`.

## Pagination

List endpoints use cursor pagination. Requests accept `limit`, `cursor`, and
`search`; responses return `items` and an optional `next_cursor`. Cursors are
server-generated and returned verbatim by clients — no offset arithmetic:

- `GET /api/v1/workspaces?organization_id=<id>`
- `GET /api/v1/organizations`
- `GET /api/v1/admin/users`
- `GET /api/v1/organizations/{id}/members`

## Endpoint map

| Area | Endpoints |
| --- | --- |
| Identity | `GET /api/v1/me`, `GET`/`PUT /api/v1/me/profile` |
| API keys | `GET`/`POST /api/v1/me/api-keys`, `DELETE .../api-keys/{key_id}` |
| Workspaces | `GET`/`POST /api/v1/workspaces`, `GET /api/v1/workspaces/{id}`, `POST .../actions/{action}` |
| Templates | `GET`/`POST /api/v1/templates`, `PUT`/`DELETE .../{id}`, `PUT .../enabled` |
| Injections | `GET`/`PUT`/`DELETE /api/v1/injections/{scope}/{scope_id}[/{key}]`, `POST .../batch-delete`, `POST /api/v1/injections/preview` |
| Access | `GET .../ssh-client-public-key`, `POST .../web-shell-tickets`, `GET`/`POST`/`DELETE .../port-mappings[/{id}]`, `POST .../open` |
| Organizations | `GET`/`POST /api/v1/organizations`, `PUT`/`DELETE .../{id}`, members, quota, usage-summary |
| Admin | `GET`/`POST /api/v1/admin/users`, `GET`/`PUT /api/v1/admin/images`, `GET`/`PUT`/`DELETE /api/v1/admin/node-pools[/{name}]`, `GET /api/v1/audit`, `GET /api/v1/admin/scaling` |
| Plugins | `GET /api/v1/plugins`, inspections, installs, configuration |
| Webhooks | `GET`/`POST /api/v1/webhooks` |
| Events | `GET /api/v1/events` (SSE) |
| System | `GET /api/v1/system/info`, `GET /api/v1/openapi.json`, `GET /livez`, `GET /readyz` |

## Events and webhooks

- **Server-sent events** — `GET /api/v1/events` streams workspace state
  changes and platform events for live dashboards.
- **Webhooks** — outbound deliveries to external HTTPS endpoints, signed so
  receivers can verify the sender.

## Errors and idempotency

Errors return a JSON body with a machine-readable code and message. Mutating
endpoints are idempotent where retries are expected (creation, actions), so
clients can safely retry on network failure.
