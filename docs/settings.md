---
id: settings
title: Settings
sidebar_label: Settings
sidebar_position: 6
---

# Settings

The console has one **Settings** navigation entry for personal preferences and
administrative controls. Older `#administration` links continue to open it.

![Unified Settings in the CI browser with sanitized MWC Demo data](/img/screenshots/settings-desktop.png)

All screenshots on this page are CI browser captures of sanitized demo data,
not production accounts or secrets.

## Find a setting

Choose **Personal**, **Organization**, or **System**, then search for a section
by its title, description, or related term. Search filters individual sections,
not just the whole category. Only sections you are authorized to use appear in
the category list or search results; a missing section may require a different
role or API-key scope. Empty results mean no accessible section matches.

![Search narrowed to the appearance and language section](/img/screenshots/settings-search.png)

Personal settings include your profile, appearance (theme and language), and API
keys. Organization settings include current organization selection and permitted
organization-level controls, including users and roles. System administrators see the system-level
management sections. See [Administration](./administration.md) for the available
management capabilities and their REST endpoints.

## Copy an API key again

In **Personal → API keys**, choose **Copy** on your own key, even if no
organization is selected. You can repeat this after refreshing the page when
the key's plaintext was retained. The list
contains metadata only; the token is fetched on the explicit Copy action and
should be stored in a password manager. Authorized administrators can copy a
different user's retained key in **Organization → Users and roles → Credential
configuration**. Access is governed by the same permissions as key management.

![Personal API-key summary in the sanitized demo; no token value is shown](/img/screenshots/user-api-keys.png)

Older hash-only keys have no recoverable plaintext. Copy cannot reconstruct
them. Create a replacement key intentionally and revoke the old one after
updating its consumers; the system does not rotate keys automatically.

## Injection settings

Workspace injection items support **environment variables and files**, as well
as SSH public keys. They are managed separately from API keys. See
[Environment variables and files](./credentials-and-files.md) for scopes and precedence.
