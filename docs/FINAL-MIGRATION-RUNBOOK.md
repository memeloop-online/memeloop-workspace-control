# Final unified-namespace migration runbook

This is an executable handoff plan, not authorization to change a cluster. Its target is the exact
namespace `memeloop-workspace-control`, with the existing installation ID
`k3si-7032544955` retained and every workspace resource name qualified by its workspace ID. The
final layout has five workspaces: four MWC workspaces plus the independent Coder TOKEN center dev
workspace. Do not start the latter cutover from itself: an external agent must perform it because
its source claim is 100 GiB and is in scope for preservation.

This runbook deliberately does not contain a namespace-deletion command or a destructive
"one-shot" script. No old namespace may be cleaned up until every acceptance and rollback-window
gate below has been signed off.

## Recorded read-only baseline (2026-09-08)

These observations are evidence, not a replacement for an immediately-pre-cutover capture using
`scripts/final-migration-readiness.sh`.

| Logical workload | Current namespace / PVC | PV / storage | Required disposition |
| --- | --- | --- | --- |
| Control plane SQLite | `mwc-k3si-7032544955` / `data-mwc-k3si-7032544955-0` (10Gi) | `pvc-317ea51a-86f8-4f1c-b2ed-2c111795c331`, `local-path`, `Delete` | Offline SQLite export/import or an explicitly reviewed local-path PV move; do not assume Longhorn procedures apply. |
| maintainance | `ws-k3si-7032544955-bd2dc9ca6aa2b1b5` / `workspace-data-w-bd2dc9ca6aa2b1b5-0` (2Gi) | `pvc-611ec96e-4bd9-4162-b237-dab2b68d3694`, Longhorn, `Delete` | Direct PV rebind only after gates. |
| tiddlywiki-dev | `ws-k3si-7032544955-b268ff46894a14b9` / `workspace-data-w-b268ff46894a14b9-0` (30Gi) | `pvc-ae9f3ccf-458d-4f3a-a1a2-97c5e2f84c96`, Longhorn, `Delete` | Direct PV rebind only after gates. |
| game-forking | `ws-k3si-7032544955-97645a0fb4771b1d` / `workspace-data-w-97645a0fb4771b1d-0` (60Gi) | `pvc-bf5e159e-bf80-47ae-bda3-58affce88008`, Longhorn, `Delete` | **Blocked:** volume was `attached/degraded`; restore healthy state and verify snapshots first. |
| rust-dev-test (MWC) | `ws-k3si-7032544955-b405d2441a1ee149` / `workspace-data-w-b405d2441a1ee149-0` (100Gi) | `pvc-fe7130d7-18a2-470e-824e-a0dea84e9a5e`, Longhorn, `Delete` | Independent MWC workspace; port 32671 is borrowed, so do not stop it until its borrower releases it. |
| Coder TOKEN center dev (independent) | `coder` / `coder-f5c0873c-8b1e-4099-b781-af5477688c39-rust-dev-home` (100Gi) | `pvc-4d799a12-262f-44ae-b3c2-c8c4cc5dfbc0`, Longhorn, `Retain` | External-agent cutover only. The observed Pod, PVC and PV claim UID all match; it is not `rust-dev-test`. |

All Longhorn entries must be `healthy`, attached only where expected, and have no outstanding
replica rebuild/fault condition at the freeze point. The target namespace being absent is expected
before cutover, not a blocker. `Delete` is unsafe during rebinding: set each selected PV reclaim
policy to `Retain` and verify it before its old PVC is removed. Record all Pod-to-PVC-to-PV,
VolumeAttachment, Longhorn Volume, snapshot, node, claim UID and manifest digests in the external
change record; do not record secret values or database contents.

## Mandatory order and gates

1. Freeze automation first. Pause Argo CD auto-sync/self-heal for both the MWC and routing
   Applications and any controller that can recreate old-namespace StatefulSets/PVCs. Merely
   scaling a workload is insufficient while its reconciliation controller remains active.
2. Resolve the two baseline blockers, perform a fresh read-only evidence capture, and create
   verified Longhorn snapshots for each workspace volume. Confirm snapshot completion and a
   tested restore path. The existing pre-start snapshots are retained but are not proof of a fresh
   cutover snapshot.
3. Promote schema **19 -> 20 only** using the published 19-to-20 bridge release. Read the
   successful CI provenance/artifact metadata and copy the exact control-plane, ttyd and other
   required immutable digests from it. This document intentionally does not invent unpublished
   digests. Verify the migration transaction and all workspace reconciliation records before the
   next release.
4. After schema-20 acceptance, begin the final offline window: pause old GitOps/reconcilers and
   stop the old control plane before running the **20 -> 21 bridge** against the offline database
   export/target database. Do not start a schema-21 coordinator in the old namespace: it would
   create a second writer while namespace-bearing records are being moved. Do not jump 19 directly
   to 21. If the schema-22 key-column release completes its test/publish gates and supplies an
   explicit accepted **20 -> 22 bridge**, it may be selected instead under a new change approval;
   do not use an untested development build. Schema compatibility is a hard gate, not a cosmetic
   version bump.
5. Keep writes frozen and stop every workspace from an external operator context, including the
   active 100Gi Coder workspace. Confirm Pods are gone, StatefulSets/controllers cannot recreate
   them, no
   VolumeAttachment remains for a volume about to move, and Longhorn reports detached before a PV
   claim is moved. Preserve all of `.codex` except explicit pod-lifetime scratch/cache directories:
   `.codex/sessions`, logs, SQLite/WAL, auth/configuration and repositories are durable data.
6. Import the offline-migrated control-plane database and workload manifests into the target
   namespace, then move all five workspace PV bindings one at a time. Start the target control
   plane only after its database, namespace-bearing records and target resource plan are ready;
   start a single workspace writer only after its binding and integrity checks pass. Do not
   simultaneously mount old and target claims.
7. Validate each workspace (PVC/PV identity, SSH host-key continuity where applicable, authenticated SSH command,
   Web Shell, retained Codex data, routes/NodePorts, and reconciler health). Keep old resources
   intact and controllers paused throughout the rollback window.
8. Only after all acceptance criteria and an agreed rollback window expire, remove old resources
   in dependency order and prove no resources remain in the old namespaces. Re-enable GitOps only
   when its desired state contains exclusively the target namespace.

## PV direct-rebind procedure (one workspace at a time)

This procedure applies to the five listed Longhorn PVs after all gates pass. The Coder PV is
handled by its external controller owner; the four MWC PVs are handled by the MWC cutover owner.
It is a human
runbook so each irreversible action receives a separate review.

1. Capture the actual Pod -> PVC -> PV -> Longhorn Volume chain and Longhorn volume state. Require
   the old PVC UID to match `PV.spec.claimRef.uid`; compare only values from the same chain. The
   two 100Gi workspaces are independent and must never be cross-compared.
2. Stop/pause the old StatefulSet and its reconciler. Wait for no old Pod, no active attachment,
   no unexpected Longhorn workload status, and a completed fresh snapshot. Verify no process can
   write the old claim.
3. Patch the selected PV reclaim policy to `Retain`; read it back. Delete only the selected old
   PVC after the retained-policy readback. Never delete the PV or Longhorn Volume.
4. Clear the PV `claimRef` under an approved, audited change, then create the target-namespace
   PVC with the exact target name, `spec.volumeName` set to that PV, compatible storage class,
   access mode, volume mode, capacity, and selector semantics. Wait for a Bound claim and confirm
   that the PV claimRef namespace/name/**new UID** match it.
5. Verify Longhorn reports precisely the intended PV and, before starting the StatefulSet/controller, that it
   is detached and healthy. Start exactly one target workload; confirm its attachment node and
   writer. Keep `Retain` throughout acceptance and the rollback window. Only after final
   acceptance, restore the reclaim policy required by the selected StorageClass (normally `Delete`
   for the four MWC `mwc-longhorn-large-delete` claims; the Coder source is already `Retain`).

Never use a copy job as a substitute for this process unless a direct rebind is rejected by a
storage owner. A copy path requires an independent checksum/metadata acceptance plan and a
separate downtime estimate.

## Control-plane SQLite handling

The control-plane PV is `local-path`, not Longhorn. Stop the control-plane and reconciliation,
then use the product's supported database `export` command to create an encrypted, mode-0600
external backup without printing its content. Validate the export structurally without exposing
values. In the target namespace, provision the reviewed target control-plane PVC/storage, deploy
the correct bridge image, and use the supported `import` command while the service is offline.
Confirm schema version, installation ID, namespace configuration, all five workspace records,
and reconciler state through redacted metadata/API checks. Keep the old control-plane PV and
encrypted export through the rollback window. A local-path PV direct rebind is allowed only with
storage-owner approval after node affinity/path, reclaim policy, claim UID, and recovery behavior
are explicitly verified.

## Cutover acceptance and rollback

The target is accepted only when: the target namespace exists; it has exactly one control plane and
five workspace identities; all PV claimRefs point to target claims with matching UIDs; each
Longhorn volume is healthy and has exactly one expected attachment; schema 21 is confirmed after
the two bridges; GitOps/Argo desired state is target-only; no old controller can reconcile; and
each workspace passes its connectivity/data checks without durable `.codex` loss.

Rollback is permitted only before old PVC bindings are irreversibly retired and while the schema
bridge rollback procedure, encrypted SQLite export, retained PV bindings, and snapshots have been
verified. Freeze the target writer, detach it, restore the recorded old claim binding or restore a
tested snapshot, and redeploy the previous compatible control-plane image/database state. Do not
roll back across an untested schema boundary and do not delete the target volume to make rollback
easier. Escalate if any schema transformation is non-reversible.

## Copy/paste task for the external cutover agent

```text
Perform the final MWC unified-namespace migration as an external operator. Do not begin from the
running Coder TOKEN center dev workspace and do not stop it from inside itself. Target exactly
namespace memeloop-workspace-control; retain installation ID k3si-7032544955 and workspace-ID-
qualified resource names. Do not deploy/delete anything until you have read
docs/FINAL-MIGRATION-RUNBOOK.md and run scripts/final-migration-readiness.sh read-only.

Current MWC source namespaces are mwc-k3si-7032544955 and ws-k3si-7032544955-{bd2dc9ca6aa2b1b5,
b268ff46894a14b9,97645a0fb4771b1d,b405d2441a1ee149}; the independent Coder source namespace is
coder. The two 100Gi claims are distinct: MWC rust-dev-test is
workspace-data-w-b405d2441a1ee149-0 -> pvc-fe7130d7-18a2-470e-824e-a0dea84e9a5e; Coder is
coder-f5c0873c-8b1e-4099-b781-af5477688c39-rust-dev-home ->
pvc-4d799a12-262f-44ae-b3c2-c8c4cc5dfbc0. Verify each Pod->PVC->PV UID chain separately. The
Coder chain is external-agent-only. game-forking's Longhorn volume was degraded and must be
healthy first. Do not expose secrets or database contents.

Pause Argo CD and every old reconciler before moving claims. Take and verify fresh Longhorn
snapshots. Release production schema 19->20 using the published bridge and validate it. For the
namespace-changing 20->21 bridge, stop the old coordinator first; migrate/import offline and do
not start schema 21 until target records/resources are ready. A tested, published, explicitly
approved 20->22 bridge may be used when schema-22 acceptance is complete; read exact image digests
from successful CI provenance rather than guessing. Stop all writers, confirm detachment,
set each selected PV to Retain, and rebind one PV at a time with matching old/new claim UID checks.
Never mount old and target claims simultaneously. Move/control SQLite only via supported encrypted
export/import while offline (or a storage-owner-approved local-path move). Preserve all .codex
except explicitly identified disposable tmp caches.

Validate Pod/PVC/PV identity, Longhorn attachments, schema, SSH host keys, SSH, Web Shell, routes,
and durable data for all five workspaces. Keep old namespaces/controllers/PVs for the agreed
rollback window. Only then make GitOps target-only and prove no old namespace resources remain.
Report commands, redacted metadata evidence, snapshots, CI digest provenance, downtime interval,
acceptance results, and any blocked gate. Never use a broad delete-namespace command.
```
