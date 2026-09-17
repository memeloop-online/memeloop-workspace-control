---
id: security-model
title: Security model
sidebar_label: Security model
sidebar_position: 9
---

# Security model

This page summarizes the guarantees the platform is designed to provide and
the responsibilities that remain with deployers.

## Isolation

- **Workload isolation** — every workspace is a separate Kubernetes workload
  with its own durable volume. Templates can select a sandboxed
  `runtime_class_name` (for example a gVisor RuntimeClass) for stronger
  kernel isolation.
- **Network isolation** — the template's `egress_policy` selects
  `unrestricted` or `internet_only`. The platform renders a NetworkPolicy
  selecting the workspace's own Pod labels, even when workspaces share a
  namespace. A namespace boundary alone is not the isolation mechanism.
- **Cluster access** — `cluster_access: false` templates give the workspace
  no Kubernetes API credentials.

## Supply chain

- **Default-deny images** — no workspace can be created from an image that a
  system administrator has not explicitly allowed (Image Contract v1).
- **Reviewed templates** — users create workspaces only from templates; they
  cannot inject arbitrary images, resources, or host paths.

## Authentication and authorization

- **RBAC** — system administrator, organization administrator, and member
  roles with distinct permissions.
- **Scoped API keys** — keys carry fine-grained scopes, mandatory expiry
  (≤ 365 days), and optional template restrictions. Plaintext keys are shown
  once at creation; only hashes are stored.
- **Short-lived tickets** — web terminal and port-mapping sessions are
  established through single-use tickets, then held in `HttpOnly`, `Secure`,
  `SameSite=Lax` session cookies.

## Data protection

- **Encryption at rest** — sensitive injection values are stored with
  AES-256-GCM envelope encryption when `MWC_ENCRYPTION_KEY` is configured.
- **Mutual TLS** — the hop from the gateway/proxy into the workspace web
  terminal validates client certificates.
- **Secret hygiene** — wildcard certificates for port mappings are held by
  the gateway only; private keys are never copied into workspace namespaces.
- **Audit** — privileged administrative actions are recorded in the audit
  log.

## Deployer responsibilities

- Inject `MWC_ENCRYPTION_KEY` and `MWC_INTERNAL_AUTH_TOKEN` from a Secret,
  not from shell history.
- Restrict template `egress_policy`, `cluster_access`, and runtime class to
  reviewed values for external tenants.
- Bind automation API keys to the specific template IDs they need.
- Keep the gateway external-auth and wildcard-certificate configuration under
  change control.
