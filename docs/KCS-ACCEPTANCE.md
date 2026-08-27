# KCS acceptance checklist

Run this checklist only against an owned KCS instance. It deliberately does not create a local
Kubernetes/K3s version matrix.

## Automated safety gates

The currently accepted amd64 images are pinned by registry digest:

- control plane: `ghcr.io/memeloop-online/memeloop-workspace-control@sha256:25f62b76f22ae4acab17ce0bd4c87de8afcc24962ed366181a3e7d972e679651`
- workspace Contract v1: `ghcr.io/memeloop-online/memeloop-workspace-control-workspace@sha256:9159f713019b3828978b8b09bc2c5bdc95c008cdc54d5e04fbcc26c7b80b1717`
- OpenSSH jump: `ghcr.io/memeloop-online/memeloop-workspace-control-ssh-jump@sha256:e4d9c26644aef7764769e4c0ae5ff8b446aa2021d483f50d465118d2b7da4824`

These are the outputs of CI run `32971286106`; do not substitute a mutable tag during acceptance.

Run the read-only preflight before Helm changes. All target-identifying values are mandatory:

```bash
export KCS_INSTALLATION_ID=public-a
export KCS_RELEASE_NAMESPACE=mwc-public-a
export KCS_STORAGE_CLASS=managed-delete
export KCS_MODE=postgresql
export KCS_PUBLIC_API=true
export KCS_PUBLIC_SSH=true
export KCS_PUBLIC_WEB_SHELL=true
export KCS_HIGRESS_NAMESPACE=higress-system
export KCS_HIGRESS_GATEWAY=higress-gateway
export KCS_HIGRESS_POD_SELECTOR=app.kubernetes.io/name=higress-gateway
scripts/kcs/preflight.sh
```

Set each `KCS_PUBLIC_*` flag to `false` for an internal installation. Gateway API CRDs and a
Higress Gateway are required only when at least one public API, SSH or Web Shell path is enabled.

Set Helm `higress.podLabels` to the same selector labels verified by the preflight.

After rollout, verify workload shape, readiness, owner labels and workspace namespace prefixes:

```bash
export KCS_EXPECT_PUBLIC_SSH=true
export KCS_EXPECT_PUBLIC_WEB_SHELL=true
scripts/kcs/verify-installation.sh
```

After an API delete reaches `deleted`, prove that the namespace and all workspace-labelled
objects are gone:

```bash
export KCS_WORKSPACE_ID=00000000-0000-0000-0000-000000000000
export KCS_WORKSPACE_NAMESPACE=ws-public-a-00000000
scripts/kcs/verify-workspace-cleanup.sh
```

The scripts do not install, patch or delete resources. Preserve their output together with the
API responses and OpenSSH command transcripts as acceptance evidence.

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
4. Delete; confirm new SSH/Web Shell authorization fails immediately, the HTTPRoute is removed,
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
terminal bytes do not traverse the control-plane API.

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
