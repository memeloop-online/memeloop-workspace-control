# gVisor workspace acceptance

The disposable API workspace is still under test; this is not production acceptance.

## Test resources

- Organization: `01a08772-8b61-7ec0-8571-d93646ea457e`
- Member user: `01a08792-5359-7220-96c8-e74af1b613c5`
- Template: `01a08792-54a4-7803-8295-7a06c104e3fb`
- Workspace: `01a08792-557e-79e0-a478-1e53dfe4b006`
- Pod: `memeloop-workspace-control/w-a4781e53dfe4b006-0`
- RuntimeClass: `gvisor-workspace-acceptance-20260909`
- Disposable SSH key directory: `/tmp/mwc-gvisor-ssh-4NG3K6`

The member's one-hour API key successfully created the workspace with only
workspace creation, read, connection, state-change and deletion scopes.
No organization/user injection references were selected. The template uses
`internet_only`, no cluster access, no BuildKit, 1 GiB Home, and the existing
Node Dev image on `serv-146231`.

A generated test public key was added through the workspace injection API
(`acceptance-ssh`, HTTP 200). Its private key stays in the restricted temporary
directory above, never in Git or Kubernetes. Remove that directory's generated
key pair when connection testing is complete.

The test template was subsequently updated through `PUT /api/v1/templates/{id}`
to `scratch_medium: memory` (HTTP 200), and a separate list request returned the
saved value. The existing workspace retains its original disk-backed template
snapshot; restarting it alone does not apply template edits. Create a replacement
test workspace from the updated template after finishing the original's connection
and persistence checks.

The key was kept in process memory only and is no longer available. For further
member-authenticated tests, mint a short-lived replacement rather than searching
logs or files for the original. Revoke test keys during final cleanup.

## Remaining checks

Startup reached Ready under `4.19.0-gvisor`. An exec check found no Kubernetes
service-account token. Strict host-key-verified SSH on port 30640 logged in as
UID/GID 1000 and wrote `/home/user/mwc-acceptance-state`; its SHA-256 is
`c97b160dc75e813e7bacd13fc312dec42c73500903c4802c2807b12659184dc9`.
The restart API returned HTTP 202 and replaced the Pod. Strict SSH reconnected
with the same pinned host key and returned the same file hash, confirming Home
data and SSH identity survived the restart.

The browser acceptance runner passed command output, terminal resize, consumed
ticket rejection, and fresh-ticket recovery on the original workspace.

From an SSH session, DNS resolved `example.com` and TCP `1.1.1.1:443` connected.
Connections to `100.64.0.1:6443`, `100.64.0.10:10250`, `10.43.0.1:443`,
and `169.254.169.254:80` did not connect within 2.5 seconds. These finite probes
do not prove all network paths or dynamic public-address isolation.

The memory-template workspace is `01a087be-5b8d-7021-bbb8-17d8c506c1cd`,
Pod `w-bbb817d8c506c1cd-0`. Its rendered build and Codex scratch volumes both
specify `Memory`. It started successfully, and host-side filesystem inspection
confirmed both mounts are tmpfs (the guest reports gVisor's proxy filesystem).
The Codex temporary-directory symlink points to `/var/lib/mwc/codex-scratch/tmp`.
BuildKit is disabled, so its cache volume is not mounted.

Before its API restart, the member UID wrote temporary scratch markers and a
persistent `/home/user/.codex/sessions/mwc-acceptance` marker with SHA-256
`75b07bb3ffb3b8ad63e79b983fbef8fd0ee8e7292144b4e7d3b57bd682074087`.
Restart returned HTTP 202. The replacement Pod reached Ready, the persistent
marker retained that hash, and both scratch markers were absent. This verifies
Pod-lifetime scratch cleanup without discarding the persistent Codex session
directory.

Continue remaining isolation and resource-pressure acceptance. Delete only
these test resources afterward, including the template, member, organization and
temporary RuntimeClass. Keep the four daily-use workspaces unchanged.
