---
id: workspaces
title: Workspaces
sidebar_label: Workspaces
sidebar_position: 3
---

# Workspaces

A workspace is an isolated, per-user Kubernetes workload created from a
reviewed template. The control plane owns its full lifecycle: provisioning,
credential injection, state transitions, reconciliation, and teardown.

![Workspace list with live resource usage](/img/screenshots/workspaces-desktop.png)

The same resource summary and lifecycle actions adapt to narrow screens:

![Responsive workspace view](/img/screenshots/workspaces-mobile.png)

## Lifecycle

Workspaces support four actions, in the console or through
`POST /api/v1/workspaces/{workspace_id}/actions/{action}`:

- `start` — provision the workspace workload and inject resolved credentials.
- `stop` — shut the workload down while keeping durable data.
- `restart` — stop and start again.
- `delete` — remove the workload and its ephemeral resources.

Workspace state changes are streamed to clients over the SSE endpoint
`GET /api/v1/events`.

## Templates

Templates define a class of workspaces. Key fields of `spec`:

| Field | Purpose |
| --- | --- |
| `image` | OCI image reference; must be on the allowlist (Image Contract v1). |
| `access_mode` | How users reach the workspace. |
| `resources` / `pod_requests` | CPU, memory, GPU, and disk allocation. |
| `workspace_user` / `workspace_home` | In-container user and home path. |
| `buildkit` | Enable an in-workspace BuildKit daemon for image builds. |
| `storage_policy.temporary_storage_gib` | Total temporary-storage capacity. |
| `cluster_access` | Whether the workspace receives Kubernetes API credentials. |
| `egress_policy` | `unrestricted` or `internet_only` network egress. |
| `runtime_class_name` | Optional Kubernetes RuntimeClass (for example a sandboxed runtime). |
| `placement` | `allowed_node_pools` and `default_node_pool` scheduling constraints. |
| `desktop` | Optional browser desktop endpoint. |

Templates can be enabled or disabled without deletion, and updated through
`PUT /api/v1/templates/{template_id}`.

## Storage

MWC separates durable user data from regenerable data:

- **Durable home** — a persistent volume mounted at the template's
  `workspace_home`. It survives stop/start and is sized independently.
- **Temporary storage** — when a scratch StorageClass is configured,
  `temporary_storage_gib` becomes the request for a Pod-owned generic
  ephemeral PVC named `workspace-scratch`. The selected class should enforce
  capacity; containers also keep small local `ephemeral-storage` requests and
  limits so writable-layer or log pressure cannot silently consume PVC
  capacity.

## Placement and node pools

Administrators define node pools (`PUT /api/v1/admin/node-pools/{name}`) that
map to Kubernetes node selectors. A template restricts scheduling through
`placement.allowed_node_pools`; users can pick an allowed pool at creation or
later through `PUT /api/v1/workspaces/{workspace_id}/placement`.
`GET /api/v1/node-pools` lists the pools available to the caller.

## Updating the image

A workspace's image can be updated in place through
`PUT /api/v1/workspaces/{workspace_id}/image`, subject to the same Image
Contract v1 allowlist. The change is reconciled on the next start.

## Runtime inspection

- `GET /api/v1/workspaces/{workspace_id}` — authoritative workspace state.
- `GET /api/v1/workspaces/{workspace_id}/runtime` — live runtime details.
- `GET /api/v1/workspace-runtimes` — runtimes across visible workspaces.
