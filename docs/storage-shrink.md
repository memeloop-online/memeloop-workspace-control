---
id: storage-shrink
title: Shrink workspace storage
sidebar_label: Storage shrink
---

# Shrink workspace storage

Shrink durable Home and CSI temporary storage without discarding their current
contents. The control plane creates smaller claims, stops the workspace, copies
offline, verifies the replacement, atomically exchanges the bindings, and restores
a workspace that was originally Ready. A workspace that was stopped stays stopped.

## Enable the feature

An administrator must build and review the dedicated helper described in
`images/storage-shrink`, then configure its immutable image digest:

```yaml
workspace:
  storageShrink:
    helperImage: registry.example/mwc-storage-shrink@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
    xattrPolicy: posix-user
    longhornNamespace: longhorn-system
```

The digest is illustrative: replace it with the actual reviewed artifact. The
workspace image is never trusted as a migration helper. No runtime dependency
installation or network download occurs in a helper Pod. Leaving the image or
the explicit xattr policy unset disables creation with `503`, without stopping
the workspace. The Helm chart grants the additional PV, StorageClass and Longhorn
read permissions only when a helper is configured.

Only filesystem CSI claims are supported. A live generic ephemeral scratch claim
is retained before its Pod is stopped, and subsequently uses an explicit binding.
Its contents therefore survive the migration and subsequent Pod replacement.
Node-local `emptyDir` is refused because stopping would destroy its contents.
To shrink scratch on a stopped workspace, a retained scratch binding must exist;
otherwise start the workspace before requesting the operation.

Supported metadata is numeric ownership, modes, nanosecond modification times,
symlinks, hardlink equivalence, POSIX ACLs and `user.*` extended attributes. Visible
unsupported xattrs fail verification. `trusted.*` cannot be certified without
elevated privileges: configuring `posix-user` is an explicit administrator
attestation that unsupported namespaces are not used. No `SYS_ADMIN` capability
is granted. Unsupported special files, inaccessible metadata, or copy errors fail
closed rather than silently excluding data.

## Start a one-click operation

Use the console's storage shrink action, review the downtime and irreversible
source-reclamation warning, and choose the desired integer GiB sizes. The console
can send `reclaim_source: true` so the durable background job finishes recovery
and source reclamation even if the browser closes.
The console entry is hidden while the workspace transitions between states
(`starting`/`stopping`); reopen it once the state settles to follow progress —
the operation itself is unaffected, and the backend rejects overlapping requests.

The API also supports retaining backups by default:

```http
POST /api/v1/workspaces/{workspace_id}/storage-shrink
Authorization: Bearer API_KEY
Idempotency-Key: unique-request-key
Content-Type: application/json

{"home_gib":20,"temporary_storage_gib":8,"expected_generation":12,"reclaim_source":true}
```

Omit either size to leave that volume's capacity unchanged. At least one target
is required. Each target must be positive, smaller than the current capacity and
strictly larger than measured offline usage. The worker uses the greater of
allocated file bytes and filesystem-used bytes, then checks the destination's
actual free bytes and free inodes. It does not impose an arbitrary percentage
headroom requirement. Sparse files and hardlinks remain sparse/hardlinked.

The endpoint returns `202` with a durable operation object. Repeat the same
idempotency key and body to replay the response; conflicting reuse returns `409`.
The workspace owner, an organization administrator or a system administrator may
request the operation with `change_workspace_state`, subject to template-key
restrictions. Status requires `read_workspace` and the same ownership/admin
boundary; unrelated organization members cannot inspect another owner's operation.

## Follow progress and understand capacity

`GET /api/v1/workspaces/{workspace_id}/storage-shrink` returns the latest operation
or `null`. It contains the operation ID, phase, error, source/destination claim
identities, used bytes/inodes and verification evidence. Phases are:

`queued → stopping → measuring → copying → verifying → switching → resuming`

In `queued`, a credential-free, no-volume Pod checks helper tools before downtime.
The StatefulSet controller must observe `Retain` for both deletion and scale-down
before validated Pod owner references are removed from source claims. Lifecycle,
placement, image and storage mutations are transactionally blocked while an
operation is active; ordinary reconciliation cannot race the migration.

With `reclaim_source: false`, success becomes `completed`, with
`sources_retained: true` and `reclaimable: true`. **This is recovery success, not
freed backend capacity.** Both old and new allocations remain until confirmation.
Verify your files and application before explicitly discarding the old volumes:

Another shrink is blocked with `409` while a completed operation retains a
reclaimable backup, so that the latest-operation status cannot hide an older
backup awaiting confirmation. Workspace deletion also cleans recorded bindings
and backups, durably capturing any missing PV identity before deleting a claim.

```http
POST /api/v1/workspaces/{workspace_id}/storage-shrink/confirm
Idempotency-Key: unique-confirmation-key
Content-Type: application/json

{"operation_id":"OPERATION_UUID"}
```

With original one-click consent or explicit confirmation, the job enters
`reclaiming`. It verifies PV UID, claimRef UID, storage class and CSI driver/handle,
changes only the verified old PV to `Delete`, and deletes the old claim using UID
preconditions. Released orphan PVs use the same identity-preconditioned cleanup.
It waits for PVC/PV disappearance and, for Longhorn, the captured backend Volume
UID to disappear. Only then is the phase `reclaimed` with
`sources_retained: false`. Longhorn `Retain` sources therefore do not silently
remain allocated while the product claims to have freed space.

## Failures and recovery

Copy/checksum failures never exchange bindings. Cleanup removes helper Pods and
their NetworkPolicy before releasing the operation guard. Before switching, a
failed migration restores source bindings and leaves the workspace stopped so it
can be restarted. Preflight failures clean up without changing its original state.
After switching, a recovery failure stops on the verified replacements and keeps
both replacements and originals: it does not throw away writes made on a new
volume by silently reverting to an older copy.

Reclamation failures return to `completed` with an error and leave the recovered
workspace running. Confirmation may be retried safely even if one old claim has
already been deleted. Reclamation never rolls back onto deleted sources.
`rolling_back` stays active until offline safety can be established; an external
mount, altered UID or changed ownership requires administrator attention rather
than unsafe cleanup. Each migration phase has a 24-hour deadline.

Helper source mounts are read-only, use no workspace injections or service-account
token, and have a distinct component selector. A validated deny NetworkPolicy is
created before execution and rechecked on reuse. NetworkPolicy rules are additive;
cluster-level allow policies and CNI enforcement remain operator responsibilities.
The dedicated helper is trusted, not an arbitrary workspace image.

Workspace deletion also removes product-managed migration bindings, retained
scratch claims and backups using their recorded identities. The older temporary
storage-size update endpoint cannot rewrite the quota of a retained explicit
binding; use the shrink operation instead.
