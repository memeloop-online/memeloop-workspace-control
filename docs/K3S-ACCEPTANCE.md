# K3S acceptance checklist

Run this checklist only against an owned K3S instance. It deliberately does not create a local
Kubernetes/K3s version matrix.

## Automated safety gates

The currently accepted amd64 images are pinned by registry digest:

- control plane: `ghcr.io/memeloop-online/memeloop-workspace-control@sha256:d17e98bc7b37d3af51d856e034b34c9c6a6561a2be5c3c6107db3c6dd18c0965`
- workspace Contract v1: `ghcr.io/memeloop-online/memeloop-workspace-control-workspace@sha256:66c2cb29d2d9c6c17c3d80113cea4ea024cc3e95f4f1ededb1e79645f45164cc`
- stock ttyd/OpenSSH client: `ghcr.io/memeloop-online/memeloop-workspace-control-ttyd@sha256:c763b36e61f12bf993a5b24da76c4b4b46cc0ed9a255b238d7ac2d24f01143bb`
- OpenSSH jump: `ghcr.io/memeloop-online/memeloop-workspace-control-ssh-jump@sha256:ccfa8d6155fab13dafab576bd37746ee9f927b1e43c8fefa7a7659516ac599b0`

The control plane is the output of CI run `33334763871`; the unchanged workspace, ttyd and jump
images retain their accepted run `33322237208` digests. Do not substitute a mutable tag during
acceptance.

The internal K3s installation accepted source revision
`d69f180f8f39e90674b72b633c6a3badda69509d` through GitOps. Argo CD reported `Synced/Healthy`,
the Pod image ID matched the control-plane digest above, and the container remained Ready with
zero restarts. Prometheus discovered the internal `auth` endpoint with `up=1` and returned the new
RSS, durable-queue and component-memory series. A controlled diagnostics rollout proved that a
Bearer token is required, CPU and heap pprof captures are usable, and all diagnostic paths return
`404` through Higress. GitOps then disabled diagnostics; the final Pod has no profiling flag or
temporary volume while `/livez`, `/readyz` and `/metrics` remain healthy.

Run the read-only preflight before Helm changes. All target-identifying values are mandatory:

```bash
export K3S_INSTALLATION_ID=public-a
export K3S_RELEASE_NAMESPACE=mwc-public-a
export K3S_STORAGE_CLASS=managed-delete
export K3S_MODE=postgresql
export K3S_PUBLIC_API=true
export K3S_PUBLIC_SSH=true
export K3S_PUBLIC_WEB_SHELL=true
export K3S_HIGRESS_NAMESPACE=higress-system
export K3S_HIGRESS_GATEWAY=higress-gateway
export K3S_HIGRESS_POD_SELECTOR=app.kubernetes.io/name=higress-gateway
scripts/k3s/preflight.sh
```

Set each `K3S_PUBLIC_*` flag to `false` for an internal installation. Gateway API CRDs and a
Higress Gateway are required only when at least one public API, SSH or Web Shell path is enabled.

Set Helm `higress.podLabels` to the same selector labels verified by the preflight.

After rollout, verify workload shape, readiness, owner labels and workspace namespace prefixes:

```bash
export K3S_EXPECT_PUBLIC_SSH=true
export K3S_EXPECT_PUBLIC_WEB_SHELL=true
scripts/k3s/verify-installation.sh
```

After an API delete reaches `deleted`, prove that the namespace and all workspace-labelled
objects are gone:

```bash
export K3S_WORKSPACE_ID=00000000-0000-0000-0000-000000000000
export K3S_WORKSPACE_NAMESPACE=ws-public-a-00000000
scripts/k3s/verify-workspace-cleanup.sh
```

The scripts do not install, patch or delete resources. Preserve their output together with the
API responses and OpenSSH command transcripts as acceptance evidence.

## Control-plane observability

1. Confirm the Pod probes call `/livez` and `/readyz`. During normal operation both return `200`;
   temporarily make only the database unavailable and confirm liveness remains `200` while
   readiness becomes `503` within two seconds.
2. Enable `monitoring.serviceMonitor.enabled`, then confirm Prometheus discovers the existing
   internal Service on named port `auth`. `/metrics` must use the OpenMetrics content type, end in
   `# EOF`, and contain HTTP count/latency/error, active request/stream, upstream, durable queue,
   process RSS, allocator and bounded component-memory families.
3. Exercise a templated workspace API route with two different workspace UUIDs. Confirm the
   resulting `mwc_http_requests_total` series contains the route template and neither UUID.
4. Set `monitoring.diagnostics.enabled=true` for a controlled incident. Through a local
   port-forward to the internal Service, verify `/diagnostics/process`, a one-second CPU capture,
   and a heap capture with the internal Bearer token. Confirm missing or invalid authorization is
   rejected and that the same paths return `404` through the public Higress URL.
5. Disable diagnostics after capture and confirm the rollout removes the profiling temporary
   volume. Do not retain profile artifacts beyond the incident evidence window.

## Installation topology

1. Install `internal-a` with SQLite and no public SSH route.
2. Install `public-a` with an independent PostgreSQL database/schema, domains, ServiceAccount,
   Secrets and public LoadBalancer IP.
3. Confirm every managed object has `workspace.memeloop.dev/owner-installation` and every
   workspace namespace is prefixed with the correct installation ID.
4. Attempt a delete with a mismatched ownership label and confirm the coordinator refuses it.

## Lifecycle and cleanup

1. Create internal and public workspaces with the standard Image Contract v1 image.
2. Wait for Ready; verify one StatefulSet replica, ClusterIP ports 2222/7681 and one RWO PVC.
3. Stop and start; confirm replicas change 1→0→1 while the PVC and OpenSSH host identity persist.
4. Delete; confirm new SSH/Web Shell authorization fails immediately, the workspace Ingress is removed,
   then Namespace, StatefulSet, Pod, Service, Secrets, ConfigMaps and PVC disappear before the
   database state becomes `deleted`.

## OpenSSH

Use the command and config returned by the API. Verify remote commands, an interactive shell,
SFTP, SCP and local/remote port forwarding through the same ProxyJump. Connect multiple
workspaces through public port 22 concurrently. Attempt to forward through one workspace login
to another workspace Service and confirm `PermitOpen` rejects it. Revoke the user's public-key
injection and confirm a new connection is rejected.

## ttyd and Higress

Issue a one-time Web Shell ticket, open the returned URL through Higress and exercise resize and
interactive input. Reload or reconnect with the consumed ticket and confirm rejection, then issue
a fresh ticket and confirm a new session succeeds. Confirm only Higress can reach port 7681 and
terminal bytes do not traverse the control-plane API. Confirm ttyd HTML assets and the WebSocket
upgrade both remain below `/shell/<short>/`; no URL prefix rewrite is allowed.

## Injection cascade

Create organization, user and inline workspace values containing blank lines, indentation,
trailing newlines, JSON/YAML/PEM and Base64 binary data. Preview provenance, verify
organization→user→workspace precedence, and confirm a locked organization key rejects both
lower-scope overrides. Create another workspace with explicit organization/user reference lists,
confirm omitted references are not materialized, and confirm a locked organization item cannot be
omitted. Inspect Kubernetes metadata/logs/audit and confirm plaintext is absent.

## PostgreSQL scale-out

Run at least three control-plane replicas. Generate concurrent workspace and Webhook jobs;
confirm `FOR UPDATE SKIP LOCKED` distributes claims, workspace leases prevent duplicate
coordination and expired leases recover. Resume SSE on another replica with `Last-Event-ID` and
confirm no gap. Enable the custom-metrics adapter and confirm CPU, memory, request-rate and
pending-job targets can all drive the HPA.

## SQLite to PostgreSQL

Export the stopped SQLite installation, import into an empty PostgreSQL database with the same
installation ID, switch the Helm mode to PostgreSQL, and verify users, organizations, workspaces,
encrypted injection values, templates, image policies, Webhooks, audit events and pending jobs.
