---
id: access
title: "Access: SSH, web terminal, port mappings"
sidebar_label: Access (SSH / terminal / ports)
sidebar_position: 5
---

# Access: SSH, web terminal, port mappings

Ready workspaces can expose SSH, a web terminal, and HTTPS port mappings. The
available controls depend on the selected template and the deployment's access
configuration.

## SSH

Internal deployments can publish a per-workspace SSH port through
`workspace.internalSshHost`. Public deployments can route connections through
the standard OpenSSH jump host in `images/ssh-jump`. Authentication uses the
caller's key pair. Copy the connection details from the workspace page. Public
connections use the jump host; internal connections use the configured host
and assigned workspace port.

Inject additional public keys into the workspace with `ssh_public_key`
injection items (see [Credentials and files](./credentials-and-files.md)).

## Web terminal

The console shows the browser terminal when the deployment has a Web Shell
origin and the workspace template enables that access path. The terminal is
backed by ttyd:

1. The client requests a short-lived, single-use ticket with
   `POST /api/v1/workspaces/{workspace_id}/web-shell-tickets`.
2. The browser connects to the workspace's ttyd endpoint, which validates the
   ticket against the control plane's internal authorization endpoint.

The hop into the workspace uses mutual TLS; the ttyd server build validates
client certificates, so only the platform proxy can reach the shell upstream.
Tickets expire quickly and cannot be reused.

## Port mappings

Port mappings are available when the deployment configures a mapping domain.
They publish an application port inside the workspace as an authenticated
HTTPS URL:

- `POST /api/v1/workspaces/{workspace_id}/port-mappings` — create a mapping
  with `internal_port` and an optional `display_name`.
- `GET /api/v1/workspaces/{workspace_id}/port-mappings` — list mappings with
  status and the stable `https_url`.
- `POST /api/v1/workspaces/{workspace_id}/port-mappings/{mapping_id}/open` —
  mint a one-time browser launch URL.
- `DELETE /api/v1/workspaces/{workspace_id}/port-mappings/{mapping_id}` —
  delete the mapping and immediately invalidate its tickets and sessions.

How it works:

- Each mapping receives a dedicated hostname
  `p-<mapping-id>.<portMappingDomain>`, routed by the Higress gateway to a
  ClusterIP Service in front of the workspace. Mappings never use NodePort or
  hostPort.
- The one-time launch URL performs a single browser exchange and then sets a
  `__Host-mwc-port-session` cookie (`HttpOnly`, `Secure`, `SameSite=Lax`).
  Subsequent requests are authorized by the gateway's external-auth check
  against the control plane.
- Deployment requires wildcard DNS for `*.<portMappingDomain>` and a wildcard
  certificate configured centrally in the Higress `credentialConfig` (with
  `fallbackForInvalidSecret`). Certificate private keys are never copied into
  workspace namespaces.
