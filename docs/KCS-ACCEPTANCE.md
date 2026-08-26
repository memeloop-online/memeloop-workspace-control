# KCS acceptance checklist

Run this checklist only against an owned KCS instance. It deliberately does not create a local
Kubernetes/K3s version matrix.

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
