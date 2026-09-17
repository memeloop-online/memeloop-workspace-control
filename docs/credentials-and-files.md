---
id: credentials-and-files
title: Credentials and files
sidebar_label: Credentials & files
sidebar_position: 4
---

# Credentials and files

MWC injects credentials and files into workspaces through **injection items**.
Items are defined once and resolved into every matching workspace at start
time, so rotating a secret does not require recreating workspaces.

![Credential and file editor](/img/screenshots/credentials-desktop.png)

## Scopes and cascade

Items exist at three scopes, applied in cascade order:

1. **Organization** — shared by every workspace in the organization.
2. **User** — personal credentials, applied to the user's workspaces.
3. **Workspace** — one-off values for a single workspace.

More specific scopes override less specific ones for the same key. Resolve the
effective set for a planned workspace with `POST /api/v1/injections/preview`
before creating it.

## Item kinds

| Kind | Result in the workspace |
| --- | --- |
| `environment_variable` | An environment variable named by `target`. |
| `secret_file` | A file at path `target` with restricted permissions. |
| `config_file` | A file at path `target`. |
| `ssh_public_key` | A public key added to the workspace's authorized keys. |

Each item has:

- `key` — unique name within its scope.
- `target` — variable name or absolute file path.
- `value` — `{ "encoding": "utf8", "value": "..." }` or
  `{ "encoding": "base64", "value": "..." }`.
- `sensitive` — protected as a write-only value. Regular configuration values
  can be reviewed, edited, and copied from the console.
- `locked` — organization administrators can lock organization items; locked
  items are always injected and cannot be deselected by workspace creators.
- `file_mode`, `owner`, `group` — optional file metadata for file kinds.
- `template_selector` and `labels` — restrict which templates the item applies
  to.

Values are stored encrypted at rest (AES-256-GCM envelope encryption) when
`MWC_ENCRYPTION_KEY` is configured.

## Managing items

```
GET    /api/v1/injections/{scope}/{scope_id}
PUT    /api/v1/injections/{scope}/{scope_id}/{key}
DELETE /api/v1/injections/{scope}/{scope_id}/{key}
POST   /api/v1/injections/{scope}/{scope_id}/batch-delete
POST   /api/v1/injections/preview
```

`scope` is `organization`, `user`, or `workspace`.

## Per-workspace selection

Workspace creation accepts `organization_injection_refs` and
`user_injection_refs`:

- Omitted or `null` — inject all items matched by template and label
  selectors.
- Empty array `[]` — inject nothing from that scope.
- Array of keys — inject only the listed items.

Locked organization items are always injected regardless of selection. The
selection is persisted atomically with the workspace and reused by previews,
reconciliation, and response attribution.
