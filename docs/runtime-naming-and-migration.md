# Runtime naming and storage migration

This document defines the compatibility boundary for prefixed resources and an optional shared
workspace Namespace. It is a design and operator runbook; it does not authorize or record a
cluster migration. Every snapshot, restore, PVC/PV binding change, GitOps change, and cluster write
requires separate, explicit approval for the named workspace or control-plane instance.

## Runtime identity and compatibility

Every workspace persists an immutable runtime identity. Reconcilers use that stored identity
instead of deriving names from the control plane's current configuration:

| Scheme | Namespace placement | StatefulSet | Home PVC |
| --- | --- | --- | --- |
| `legacy_v1` | Dedicated `ws-<installation>-<short-id>` | `workspace` | `workspace-data-workspace-0` |
| `prefixed_v2` dedicated | Dedicated `ws-<installation>-<short-id>` | `w-<short-id>` | `workspace-data-w-<short-id>-0` |
| `prefixed_v2` shared | configured shared Namespace | `w-<short-id>` | `workspace-data-w-<short-id>-0` |

`workspace.sharedNamespace` / `MWC_WORKSPACE_SHARED_NAMESPACE` affects only newly created
`prefixed_v2` workspaces. An absent variable keeps dedicated placement. The Helm default is an
empty value and deliberately omits the variable, which has that same effect. A directly supplied
empty environment variable is different: Clap parses it as a configured empty string and startup
validation rejects it. A non-empty value must be one lower-case DNS label of at most 63 characters;
it is not a dotted DNS name.

The shared Namespace is installation-owned, never workspace-owned. Reconciliation creates it when
it is absent, with `workspace.memeloop.dev/owner-installation=<installation>`, or verifies that an
existing Namespace has exactly that installation owner and no workspace-ID owner before applying
workspace resources. Before database insertion, the creation API reads an existing configured
Namespace and rejects it unless those ownership conditions match; a missing Namespace is allowed
because the coordinator will create it. Apply repeats the ownership check to close the race between
preflight and reconciliation. RBAC, quota/policy, and StorageClass failures remain ordinary
fail-closed reconciliation errors; the ownership preflight is not presented as a Kubernetes
dry-run or a guarantee that every later apply will succeed.

The five existing production workspace PVCs are a fixed legacy set. All five must retain their
current dedicated Namespace, `legacy_v1` identity, `workspace` StatefulSet name, and
`workspace-data-workspace-0` PVC name. A schema backfill records that existing layout; enabling a
shared Namespace must not rename, recreate, adopt, delete, or relocate any member of the five-PVC
set. Before any future operational work, capture the exact five workspace IDs, Namespaces, PVC/PV
UIDs, capacities, StorageClasses, CSI volume handles, and attachment state from an approved
read-only inventory and require the post-change inventory to match.

Prometheus storage and node rules select running workspace Pods through the currently allowlisted
`workspace.memeloop.dev/owner-installation` and `workspace.memeloop.dev/workspace-id` Pod labels.
The Home rule joins those Pods to
`kube_pod_spec_volumes_persistentvolumeclaims_info`, then filters kubelet PVC statistics by the
resulting `(namespace, persistentvolumeclaim)` pairs. The workspace-ID value is used only on the
right side of a set join: recording rules and alerts do not copy it into their output. Home alerts
retain only the Namespace and PVC labels needed to identify the affected claim.

This rule covers legacy, prefixed dedicated, and prefixed shared workspaces only while a Pod object
exists. A stopped workspace has no Pod and therefore intentionally has no
`mwc_workspace_home_used_percent` series or Home-capacity alert. Before relying on the rule, use a
known running workspace to verify that kube-state-metrics exposes both required Pod labels and the
Pod/PVC relationship metric and that the join yields its one Home claim. During a stopped migration
window, inspect the recorded PVC and storage-backend volume directly; absence from this recording
rule is not evidence that the stopped Home is healthy or absent.

## Why a PVC cannot be moved directly

A PVC is namespaced. Kubernetes has no operation that changes a PVC's Namespace, and copying its
manifest does not move its bound storage. A StatefulSet rename also changes the generated claim
name. Never patch a live claim, edit a bound PV's `claimRef`, or let two Pods mount a single-writer
Home volume concurrently to imitate a move.

If a specific workspace is ever approved for relocation, perform exactly one workspace at a time.
All three strategies below first require the same fence:

1. Record the immutable runtime identity and the complete PVC/PV/CSI binding and rollback
   inventory. Confirm no unrelated workspace is in the change set.
2. Stop the workspace through MWC, then independently prove its Pod is absent, the PVC has no
   consumer, and the volume is detached. Reject the window if a client or controller restarts it.
3. Create and wait for a ready storage snapshot. Run the approved read-only filesystem and SQLite
   integrity checks against a stopped copy; record aggregate results without secrets or filenames.
4. Choose exactly one approved strategy below. Change persisted runtime identity only through a
   purpose-built, reviewed migration operation in the same stopped window; ordinary configuration
   changes must never reinterpret it. The current product does not automate any of these three
   storage migrations.
5. Reconcile while the workspace remains stopped, then verify ownership labels, selectors, routes,
   Services, Secrets, and the exact target claim before starting it.
6. Start and accept that workspace before proceeding to another. Keep every strategy-specific
   recovery source until the owner accepts the result.

The identity changes are deliberately different:

| Strategy | Namespace identity | PVC identity | PV UID | CSI `volumeHandle` |
| --- | --- | --- | --- | --- |
| Same Namespace, direct claim | Same name and UID | Same name and UID | Unchanged | Unchanged |
| Cross Namespace, snapshot/restore | Target has a different Namespace name and its own existing or newly created UID; source remains | New target name and UID; source remains | New target PV UID under normal dynamic restore | New target handle under normal dynamic restore |
| Cross Namespace, retained-PV prebind | Target has a different Namespace name and UID; source Namespace remains | Source PVC is deleted; target has a new name and UID | Unchanged | Unchanged |

The snapshot/restore row describes normal CSI dynamic restore behavior; the recorded driver output
is authoritative if a backend uses different identity semantics.

### Same Namespace: mount the original PVC directly

This is not a PVC move. A replacement workload in the same Namespace may explicitly reference the
existing claim by `claimName`, so Namespace name/UID, PVC name/UID, PV name/UID, and CSI
`volumeHandle` all remain unchanged. Do not render a second StatefulSet claim template for that
volume. Because the current workspace runtime model derives its claim from a StatefulSet template
and has no per-workspace existing-claim option, this strategy requires a separately reviewed
product capability; changing naming configuration alone cannot select it.

Rollback stops and detaches the replacement workload, restores its former runtime/workload
reference, and remounts the same claim. No binding object is recreated, so all four identities must
still match the pre-change record. Snapshot target writes before rollback if they must be retained.

### Cross Namespace: snapshot and restore

Restore the stopped source snapshot into a new PVC in the target Namespace and verify aggregate
contents before cutover. The target Namespace has its own UID (new if created for the migration,
unchanged if it is an already approved shared Namespace); the target PVC has a new name and UID;
dynamic provisioning normally creates a new PV UID and a new CSI `volumeHandle`. Record the storage
driver's actual restore identities rather than assuming them. The original Namespace, PVC UID, PV
UID, and CSI handle remain unchanged and detached as the rollback source.

Rollback stops and detaches the target, snapshots it if it received writes, restores the old runtime
identity, and starts the still-bound original claim only after integrity verification. The original
copy is stale after target writes; preserving those writes requires a separately approved reverse
restore or application-aware data transfer. A rollback must not silently claim that starting the
old PVC preserves post-cutover writes.

### Cross Namespace: retain and prebind the original PV

Set and verify the source PV reclaim policy and every applicable controller retention control as
`Retain`. After the source is stopped and detached, delete the source PVC normally, wait for the PV
to become `Released`, clear only its recorded stale `claimRef`, and create the exact target PVC with
`volumeName` prebinding. The target Namespace has a different UID; the source PVC UID is destroyed;
the target PVC has a new UID; the PV UID and CSI `volumeHandle` must remain byte-for-byte unchanged.
Any mismatch blocks startup. This strategy has the narrowest and most dangerous binding window.

Rollback is not “re-enable the old PVC”: that PVC object no longer exists. Stop and detach the
target, snapshot any target writes, delete the target PVC while the PV remains `Retain`, wait for
`Released`, clear only the verified target `claimRef`, and recreate/prebind the original PVC name in
the original Namespace. The recreated source claim has a new PVC UID; the PV UID and CSI handle
remain unchanged. Verify those facts and filesystem integrity before restoring the old runtime
identity and starting it. Never run source and target consumers together.

Neither cross-Namespace strategy is implicitly authorized by the shared-Namespace option.

## Control-plane SQLite volume

The SQLite control plane is a separate migration boundary. The chart sets the StatefulSet
`persistentVolumeClaimRetentionPolicy` to `Retain` for deletion and scale-down, and
`sqlite.existingClaim` can mount a pre-created claim instead of rendering a claim template. These
safeguards do not migrate data and do not override PV reclaim policy, Namespace deletion, or an
Argo CD prune.

For a new installation, `sqlite.existingClaim` can directly select a prepared claim in the release
Namespace. For an existing installation, `volumeClaimTemplates` is an immutable StatefulSet field:
an upgrade cannot patch the existing StatefulSet in place from a generated claim to
`sqlite.existingClaim`. Use a separately approved, controlled recreation instead:

1. First deploy with `sqlite.existingClaim` empty, let the Retain policy reach the live StatefulSet,
   and independently verify both `whenDeleted: Retain` and `whenScaled: Retain` plus the PV reclaim
   policy. Do not combine this verification with the immutable-field change.
2. Freeze every controller that can restore the writer. Pause Argo CD automated sync, self-heal,
   and prune for the Application, record the live and desired revisions, and verify that no other
   reconciler will recreate or scale the StatefulSet. An alternative is a separately reviewed,
   declaration-first maintenance revision that renders `replicas: 0`; wait for that revision to be
   Synced before continuing. A manual scale while self-heal remains active is not a writer fence.
3. Stop the sole replica, prove there are no writers, account for WAL state, run the integrity
   check, and record the StatefulSet, PVC, PV, and CSI identities.
4. Delete and recreate only the StatefulSet under the fenced Argo/Helm window. Do not delete the
   generated PVC. Render the replacement with `sqlite.existingClaim` equal to that retained claim
   and no `volumeClaimTemplates`.
5. Confirm the replacement Pod mounts the recorded PVC/PV/CSI handle and passes integrity and
   readiness checks. Only then restore the recorded automated-sync, self-heal, and prune settings
   and verify the Application remains Synced and Healthy.

A control-plane Namespace or claim change requires one of these stopped-writer procedures:

- stop the only control-plane replica, confirm there are no database writers, checkpoint or
  otherwise account for WAL state, run SQLite `PRAGMA integrity_check`, detach the volume, perform
  the approved retained-volume/snapshot restore, and repeat the integrity check before startup; or
- create a transactionally consistent SQLite backup while the source is controlled, restore it to
  a pre-created target claim, and verify integrity and the immutable installation identity before
  starting the target.

Do not use a live filesystem copy of the database, its `-wal`, or its `-shm` file. Keep the envelope
key and installation ID unchanged. `sqlite.existingClaim` must name a claim in the Helm release
Namespace; it cannot reference a claim across Namespaces. It is valid only in SQLite mode and is
mutually exclusive with `sqlite.storageClassName` because provisioning is owned outside the chart.

For control-plane rollback, first restore the same Argo writer fence: pause automated sync,
self-heal, and prune (or apply and verify the declaration-first zero-replica maintenance revision),
then stop the target. If it accepted writes, take a consistent backup or stopped snapshot.
Recreating the original VCT-shaped StatefulSet does not adopt an arbitrarily named claim: the
retained claim must still have the exact generated name that the template expects. Restore the
recorded manifest/binding, run the integrity check, verify the installation identity, and then
start exactly one replica. Restore automatic reconciliation only after readiness and binding
identity are accepted.

## Argo CD prune window

Automated prune can turn an otherwise reversible storage change into deletion. Removing a
`volumeClaimTemplates` entry, replacing a StatefulSet, moving an Application destination
Namespace, or removing the old Namespace from Git can make the old StatefulSet, claim, or entire
Namespace eligible for pruning. StatefulSet claim-retention policy is not protection against
direct PVC or Namespace deletion, Application finalizers, or a storage backend whose reclaim
policy deletes the volume.

Any later approved cross-Namespace rollout needs a reviewed, two-phase GitOps plan: first add and
verify the target while the source and rollback objects remain declared and pruning is explicitly
prevented for the window; only after integrity, binding, application, and owner acceptance may a
separately approved change retire the source. The same-name control-plane StatefulSet conversion
from a claim template to `sqlite.existingClaim` cannot declare source and target StatefulSets
simultaneously; it instead requires the writer-fenced recreation sequence above. In either case,
record the exact Argo revision, automated-sync/self-heal/prune settings, rendered diff, prune set,
resource UIDs, PV reclaim policies, snapshots, and rollback revision before the window. A rollback
revision is not sufficient if automation can restart a writer or prune has already deleted the only
recoverable storage.

This repository change intentionally performs no GitOps edit, cluster write, snapshot, PVC move,
or migration of the five legacy workspace claims or the control-plane database.
