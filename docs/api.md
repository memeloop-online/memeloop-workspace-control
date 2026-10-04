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

Keys are managed from **Administration → Users and roles → Edit user → API
keys**, or through `GET`/`POST /api/v1/me/api-keys` and `DELETE
/api/v1/me/api-keys/{key_id}`. Rules:

- Every key carries at least one fine-grained scope.
- Expiry is a Unix timestamp at most 365 days in the future.
- Authorized system administrators with both `manage_system` and
  `manage_api_keys` can copy a user's retained plaintext key from its editor;
  self-service list responses remain summary-only.
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
| Workspaces | `GET`/`POST /api/v1/workspaces`, `GET /api/v1/workspaces/{id}`, `POST .../actions/{action}`, `PUT .../temporary-storage` |
| Templates | `GET`/`POST /api/v1/templates`, `PUT`/`DELETE .../{id}`, `PUT .../enabled` |
| Injections | `GET`/`PUT`/`DELETE /api/v1/injections/{scope}/{scope_id}[/{key}]`, `POST .../batch-delete`, `POST /api/v1/injections/preview` |
| Access | `POST .../web-shell-tickets`, `GET`/`POST`/`DELETE .../port-mappings[/{id}]`, `POST .../open` |
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

System administrators can resize a stopped workspace's temporary storage with
`PUT /api/v1/workspaces/{workspace_id}/temporary-storage`, providing
`temporary_storage_gib` (1–2,048) and `expected_generation`. Include an
`Idempotency-Key`; the accepted snapshot and accounting update are reconciled
on the next start.
## Adopt or bind an existing workspace Home PVC

`PUT /api/v1/workspaces/{workspace_id}/home-pvc` requires an unrestricted system
administrator with `manage_system`, an `Idempotency-Key`, and this JSON body:

```json
{
  "namespace": "memeloop-workspace-control",
  "claim_name": "migrated-home-20gi",
  "claim_uid": "the-current-pvc-uid",
  "capacity_gib": 20,
  "expected_generation": 13
}
```

The binding is workspace-specific `home_volume_binding`, never a reusable template
option. The namespace must equal the workspace runtime namespace. The claim must
exist, be Bound and non-terminating, and have the requested UID, filesystem mode,
and matching requested/actual whole-GiB capacity. Claims reserved by other
workspaces or referenced by another workload are rejected. Kubernetes API failure
fails closed. This endpoint does not create, resize, delete, or move a PVC.

Ordinary binding requires `Stopped`, no workspace Pod, and no Pod mounting the
target claim. A `Ready` workspace may instead **adopt its current Home**: its live
StatefulSet and Ready Pod must already mount that exact claim, with matching MWC
ownership, Pod controller UID, Home mounts, and no volume claim templates. Ready
adoption cannot switch to another volume. It rejects running reconcile jobs and
held workspace leases. Pending jobs do not block adoption: after the generation
CAS, they reload the workspace and use its persisted binding, not an old volume
snapshot. Do not stop an unadopted migrated workspace just to use this API.

An unpersisted custom Home blocks StatefulSet mutation during reconciliation and
defers the job without consuming failure attempts or changing workspace state.
If terminal job failure has marked the workspace `Failed`, this endpoint permits
the same adoption only when the live workload passes every Ready adoption check.
It atomically restores `Ready` with the binding and generation update, without
queueing reconciliation or changing the live volume. Running reconcile
jobs, a held workspace lease, and stale generations still reject recovery.

The operation holds the existing workspace lease and applies a generation CAS.
One database transaction persists namespace/name/UID/capacity, corrects both the
workspace template snapshot and accounting `disk_gib`, increments generation, and
writes an audit record. Stopped binding queues reconciliation; Ready adoption
does not queue it or directly modify the StatefulSet. An already-pending reconcile
may subsequently update the Pod template generation annotation and cause a rolling
restart; adoption does not guarantee zero downtime. New Pods still use the
persisted Home binding, never the old generated claim. GET workspace and runtime
allocation show the bound capacity;
runtime metrics resolve the bound claim by name and UID rather than the obsolete
generated claim. Missing/replaced claims never fall back to the old Home.

Responses: `200` success or idempotent replay; `400` invalid binding; `403` role,
scope, or template-restricted key; `409` generation/lease/state/ownership/reference
conflict; `422` missing or mismatched PVC; `502/503` unavailable Kubernetes. An
external Home PVC is retained when its workspace is deleted. Release both API and
reconciler support together before using this endpoint.
