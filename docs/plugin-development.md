---
id: plugin-development
title: Plugin development
sidebar_label: Plugin development
sidebar_position: 8
---

# Plugin development

MWC plugins are WebAssembly components that extend the control plane with
creation policies, API middleware, custom API routes, and console UI surfaces.
Plugins run sandboxed; all host interaction crosses a versioned WIT interface.

## Interface

The contract is `wit/workspace-control.wit` (package
`memeloop:workspace-control@0.2.0`). A plugin exports the `plugin-backend`
interface:

| Export | Called when | Returns |
| --- | --- | --- |
| `admit-create(context, plan)` | A workspace is about to be created | `decision` — allow or deny with a predeclared code |
| `check-request(context)` | An API request matches the plugin's middleware | `decision` |
| `handle-api(request)` | A request hits the plugin's own API route | `api-response` — status, content type, body |

Deny codes must be predeclared in `plugin.json`; no guest free text crosses
the ABI. Plugins declaring `wit_version` within `>=0.1.0, <0.3.0` are
supported.

## Package layout

A plugin package is a directory (distributed as an archive) containing:

```
my-plugin/
  plugin.json     # manifest, required
  plugin.wasm     # compiled component, optional for pure-UI packages
  assets/         # optional static assets for UI surfaces
```

Manifest fields (`plugin.json`):

| Field | Purpose |
| --- | --- |
| `id`, `name`, `version`, `description` | Identity and display metadata. |
| `wit_version` | Interface version range the plugin targets. |
| `wasm` | File name of the component, if any. |
| `workspace_create_policy` | Enable `admit-create` for workspace creation. |
| `denial_codes` | Predeclared codes the plugin may return. |
| `configuration` | JSON Schema and default value for admin-editable configuration. |
| `assets` | Static assets served to the console. |
| `ui_surfaces` | Console placements the plugin renders into. |
| `api_routes` | Routes served under `/api/v1/plugin-api/{plugin_id}/{route_id}/`. |
| `api_middleware` | Request matchers that trigger `check-request`. |

Package limits: manifest ≤ 1 MiB, component ≤ 64 MiB, archive ≤ 80 MiB and
≤ 64 files.

## Installation and lifecycle

Plugins are installed by system administrators through a two-phase flow:

1. **Inspection** — upload the archive or point at a URL/GitHub release; the
   control plane validates the package and returns what it will change:
   - `POST /api/v1/plugins/inspections/upload`
   - `POST /api/v1/plugins/inspections/url`
   - `POST /api/v1/plugins/inspections/github-release`
2. **Confirmation** — `POST /api/v1/plugins/installs` with the inspection
   token completes the install.

Afterward: `PUT /api/v1/plugins/{plugin_id}/enabled` toggles the plugin,
`DELETE /api/v1/plugins/{plugin_id}` uninstalls it, and
`GET`/`PUT`/`DELETE /api/v1/plugins/{plugin_id}/configuration` manages its
configuration (validated against the declared schema).

## UI surfaces and API routes

- Console surfaces are created per session with
  `POST /api/v1/plugins/{plugin_id}/ui-surfaces/{surface_id}/sessions`; assets
  are served under `/api/v1/plugin-ui/{plugin_id}/{session_id}/` and the
  surface talks to its backend through the session bridge
  (`POST .../bridge`).
- Plugin API routes are invoked under
  `/api/v1/plugin-api/{plugin_id}/{route_id}/{*path}` with a 256 KiB request
  body limit.

## Versioning guidance

- Keep `wit_version` pinned to the interface you tested against.
- Treat `denial_codes` as part of your public API; users see them.
- Validate `configuration-json` defensively — administrators can edit it.
