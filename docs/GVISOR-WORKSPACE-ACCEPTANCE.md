# gVisor workspace acceptance

The disposable API workspace is still under test; this is not production acceptance.

## Test resources

- Organization: `01a08772-8b61-7ec0-8571-d93646ea457e`
- Member user: `01a08792-5359-7220-96c8-e74af1b613c5`
- Template: `01a08792-54a4-7803-8295-7a06c104e3fb`
- Workspace: `01a08792-557e-79e0-a478-1e53dfe4b006`
- Pod: `memeloop-workspace-control/w-a4781e53dfe4b006-0`
- RuntimeClass: `gvisor-workspace-acceptance-20260909`

The member's one-hour API key successfully created the workspace with only
workspace creation, read, connection, state-change and deletion scopes.
No organization/user injection references were selected. The template uses
`internet_only`, no cluster access, no BuildKit, 1 GiB Home, and the existing
Node Dev image on `serv-146231`.

The key was kept in process memory only and is no longer available. For further
member-authenticated tests, mint a short-lived replacement rather than searching
logs or files for the original. Revoke test keys during final cleanup.

## Remaining checks

Confirm startup, SSH and browser terminal interaction, PVC persistence across API
restart, and memory-backed scratch after upgrading the test template. Delete only
these test resources afterward, including the template, member, organization and
temporary RuntimeClass. Keep the four daily-use workspaces unchanged.
