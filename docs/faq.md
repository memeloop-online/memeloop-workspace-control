---
id: faq
title: FAQ
sidebar_label: FAQ
sidebar_position: 10
---

# Frequently asked questions

## Why can't I create a workspace from my image?

Images are default-deny. A system administrator must allow the image through
the admin console or `PUT /api/v1/admin/images` before any template can
reference it.

## Why can't I create a workspace at all?

Either no template is enabled, or your quota is exhausted. Check with your
organization administrator; quotas exist at both organization and user level.

## I rotated a secret. Do I need to recreate workspaces?

No. Injection items are resolved at workspace start. Update the item at its
scope and restart the workspace to pick up the new value.

## Can a workspace creator opt out of an organization credential?

Only if the item is not locked. Locked organization items are always injected;
unlocked items can be deselected with `organization_injection_refs` at
creation time.

## Is the API key retrievable later?

Yes, for an administrator with both `manage_system` and `manage_api_keys`.
Open **Administration → Users and roles**, edit the user, then choose the
**API keys** tab and use **Copy**. Ordinary self-service API-key lists expose
summaries only. Historical keys created before plaintext retention cannot be
reconstructed; create a replacement for those keys.

## Does the port mapping expose my app publicly?

No. Each mapping requires a one-time launch exchange and a session cookie
validated by the gateway's external-auth check against the control plane.
Deleting the mapping invalidates existing sessions immediately.

## Where is the full API contract?

Each deployment serves `GET /api/v1/openapi.json`. The
[API reference](./api.md) summarizes the integration model.

## Can I extend the platform without forking it?

Yes. [Plugins](./plugin-development.md) can enforce creation policies,
intercept API requests, add API routes, and render console UI surfaces, all
over the versioned WIT interface.
