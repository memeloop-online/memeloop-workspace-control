# gVisor workspace acceptance

The formal-RuntimeClass API workspace is still under network test; this is not
external-tenant production acceptance.

## Test resources

- Organization: `01a08772-8b61-7ec0-8571-d93646ea457e`
- Member user: `01a08792-5359-7220-96c8-e74af1b613c5`
- Template: `01a08792-54a4-7803-8295-7a06c104e3fb`
- Workspace: `01a08815-fa3a-78b0-9ebf-c0a07822806e`
- Pod: `memeloop-workspace-control/w-9ebfc0a07822806e-0`
- RuntimeClass: `gvisor` (retain during test cleanup)
- Disposable SSH key directory: removed after connection tests

The original workspace `01a08792-557e-79e0-a478-1e53dfe4b006` has been deleted
through the product API (HTTP 202). Its Pod, StatefulSet, Services, ConfigMaps,
Secrets, NetworkPolicies, Ingresses and PVC are absent; its PV is also gone.
A subsequent workspace GET returned HTTP 404. Only disposable test data was
removed. The subsequent memory-backed workspace was also deleted after startup
policy-refresh acceptance. Only the formal-class workspace and shared test
organization/user/template remain.

The member's one-hour API key successfully created the workspace with only
workspace creation, read, connection, state-change and deletion scopes.
No organization/user injection references were selected. The template uses
`internet_only`, no cluster access, no BuildKit, 1 GiB Home, and the existing
Node Dev image on `serv-146231`.

A generated test public key was added through the workspace injection API
(`acceptance-ssh`, HTTP 200). Its private key was held only in a restricted
temporary directory, never in Git or Kubernetes, and was deleted after testing.

The test template was subsequently updated through `PUT /api/v1/templates/{id}`
to `scratch_medium: memory` (HTTP 200), and a separate list request returned the
saved value. The existing workspace retains its original disk-backed template
snapshot; restarting it alone does not apply template edits. The subsequent
replacement test workspace captured the updated memory-backed template.

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

Application mapping acceptance used the formal workspace after its ttyd sidecar
was upgraded to the authenticated port proxy. The original application port
was unreachable from another workspace, while port 8443 rejected a client
without the gateway certificate. Higress rejected an unauthenticated request,
accepted a one-use bootstrap session, and rejected ticket replay. Adding and
removing mappings reloaded and cleared the exact route without replacing the
workspace Pod.

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

## Resource and credential boundary

The memory-backed workspace's member process has `CapEff=0` and `NoNewPrivs=1`.
Docker/containerd sockets and the Kubernetes service-account token are absent.
The guest reports `Seccomp=0`; this is gVisor's emulated process view and is not
evidence that the host sentry's seccomp protection is disabled.

The host Pod cgroup limits are 0.6 CPU, 1152 MiB memory and 1024 PIDs, including
the ttyd sidecar. Two CPU workers were terminated after five seconds; host
`nr_throttled` increased from 4 to 21 and `throttled_usec` from 67131 to 710882.
The command completed normally. This verifies actual CPU throttling on this Pod,
not per-process quotas or all resource exhaustion scenarios.

A bounded guest-process test spawned 1100 short-lived `/bin/sleep` children
from an unprivileged Node.js process. All 1100 spawned, no spawn errors were
reported, and the runner reaped its children and returned normally. Both workspace
containers remained Ready without restarts. The host Pod cgroup reported no
PID-limit hits and no OOM events during the test.

Therefore the host `pids.max=1024` must not be described as a 1024-process limit
inside the guest. gVisor multiplexes guest execution onto host tasks; these are
different accounting boundaries. This bounded check demonstrates recovery, not
protection against every process-exhaustion workload. CPU and memory limits at
the host sandbox boundary remain necessary. See the upstream
[resource model](https://gvisor.dev/docs/architecture_guide/resources/) and
[compatibility guidance](https://gvisor.dev/docs/user_guide/compatibility/).

## Missing-runtime failure

A disposable template referenced nonexistent RuntimeClass
`mwc-intentionally-unavailable`. The rendered StatefulSet retained that name,
and Kubernetes rejected Pod creation with `RuntimeClass ... not found`. No Pod
ran with runc. The workspace response had no SSH connection, and its Web Shell
ticket endpoint returned HTTP 409.

The test workspace `01a087da-c992-75a3-90d3-74da722f6aed` was deleted through the
API; its StatefulSet and PVC are gone. The temporary template was disabled and
deleted (HTTP 204). No RuntimeClass was created for this negative test.

## Formal RuntimeClass

Argo CD now manages RuntimeClass `gvisor` from the internal GitOps repository,
under `apps/memeloop-workspace-control/runtime`. Its handler is `runsc` and its
node selector requires `sandbox.memeloop.dev/gvisor-ready=true`.

The test template was changed to reference this class. New test workspace
`01a08815-fa3a-78b0-9ebf-c0a07822806e` (Pod `w-9ebfc0a07822806e-0`) first had a
bound PVC but remained Unschedulable: no node matched the runtime selector and
template node affinity. It did not run with runc. After the accepted node
`serv-146231` was labeled, the same Pod scheduled there and both containers
became Ready. The guest kernel reports `4.19.0-gvisor`; the ordinary `user`
account is UID 1000 with `CapEff=0` and `NoNewPrivs=1`, without a mounted
Kubernetes service-account token.

No daily-use workspace changed runtime. Node registration confirms runtime
availability, not completed external-tenant network isolation. Keep the formal
class and registered node when cleaning up the disposable acceptance workspace;
remove only the temporary acceptance class after its last Pod is gone.

Cleanup after the startup-policy test removed workspace
`01a087be-5b8d-7021-bbb8-17d8c506c1cd` through the API (HTTP 202). Its
StatefulSet, Pod, Services, Secrets, ConfigMaps, NetworkPolicy, Ingresses and PVC
are absent. No Pod referenced `gvisor-workspace-acceptance-20260909`, so that
temporary RuntimeClass was deleted. The generated SSH key pair and known-hosts
file were deleted from their dedicated temporary directory. These were
disposable test artifacts; no daily workspace or formal RuntimeClass was removed.
