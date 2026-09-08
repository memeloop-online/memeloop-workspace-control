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
2. Resolve the MWC baseline blocker, perform a fresh read-only evidence capture, and create
   verified Longhorn snapshots for the four MWC volumes. Confirm snapshot completion and a tested
   restore path. The independent Coder volume remains live and is explicitly out of this first
   phase. The existing pre-start snapshots are retained but are not proof of a fresh cutover
   snapshot.
3. Promote schema **19 -> 20 only** using the published 19-to-20 bridge release. Read the
   successful CI provenance/artifact metadata and copy the exact control-plane, ttyd and other
   required immutable digests from it. This document intentionally does not invent unpublished
   digests. Verify the migration transaction and all workspace reconciliation records before the
   next release.
4. After schema-20 acceptance, begin the final offline window: pause old GitOps/reconcilers and
   stop the old control plane. Use the current, verified-and-published schema-22 release to run
   the offline **20 -> 22** migration; it includes both required transformations, so no
   schema-21 coordinator/process is started. Do not jump 19 directly to 22. A snapshot can be
   imported only into an empty target already at the same schema version, so migrate before export
   and before import. If the schema-22 CI/provenance or publication gate is not green, cutover is
   prohibited. Schema compatibility is a hard gate, not a cosmetic version bump.
5. Keep MWC writes frozen and stop the four MWC workspaces from an external operator context.
   The independent 100Gi Coder workspace has no MWC database writer relationship: leave it
   running and do not migrate or stop it in this phase. Confirm the four MWC Pods are gone,
   their StatefulSets/controllers cannot recreate them, and no
   VolumeAttachment remains for a volume about to move, and Longhorn reports detached before a PV
   claim is moved. Preserve all of `.codex` except explicit pod-lifetime scratch/cache directories:
   `.codex/sessions`, logs, SQLite/WAL, auth/configuration and repositories are durable data.
6. Import the offline-migrated control-plane database and four MWC workload manifests into the
   target namespace, then move those four MWC PV bindings one at a time. Start the target control
   plane only after its database, namespace-bearing records and target resource plan are ready;
   start a single MWC writer only after its binding and integrity checks pass. Do not simultaneously
   mount old and target claims.
7. Validate the four MWC workspaces (PVC/PV identity, SSH host-key continuity, authenticated SSH
   command, Web Shell, retained Codex data, routes/NodePorts, and reconciler health). Re-enable the
   target MWC GitOps/reconciler only after it has target-only desired state. Keep the old MWC
   resources intact throughout their rollback window.
8. This completes the MWC platform phase. The independent Coder migration is a later external-only
   single-workspace change; it must not interrupt the healthy four MWC workspaces. The final global
   cleanup gate applies only after that second phase is accepted.

## PV direct-rebind procedure (one workspace at a time)

This procedure applies to the four MWC Longhorn PVs in the first phase. The Coder PV is excluded
and is handled only by its external controller owner in the later second phase. It is a human
runbook so each irreversible action receives a separate review.

1. Capture each MWC Pod -> PVC -> PV -> Longhorn Volume chain and Longhorn volume state. Require
   the old PVC UID to match `PV.spec.claimRef.uid`; compare only values from the same chain. The
   independent Coder 100Gi chain is not part of this operation and must never be cross-compared.
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
   for these four MWC `mwc-longhorn-large-delete` claims).

Never use a copy job as a substitute for this process unless a direct rebind is rejected by a
storage owner. A copy path requires an independent checksum/metadata acceptance plan and a
separate downtime estimate.

## Control-plane SQLite handling

The control-plane PV is `local-path`, not Longhorn. Stop the control-plane and reconciliation,
then use the product's supported database `export` command to create an encrypted, mode-0600
external backup without printing its content. Validate the export structurally without exposing
values. In the target namespace, provision the reviewed target control-plane PVC/storage, deploy
the correct bridge image, and use the supported `import` command while the service is offline.
Confirm schema version, installation ID, namespace configuration, all four MWC workspace records,
and reconciler state through redacted metadata/API checks. Keep the old control-plane PV and
encrypted export through the rollback window. A local-path PV direct rebind is allowed only with
storage-owner approval after node affinity/path, reclaim policy, claim UID, and recovery behavior
are explicitly verified.

## Second phase: external Coder single-workspace handoff

After the MWC platform phase has passed, keep its four target workspaces running normally. Only an
external agent may schedule the Coder change. It must independently snapshot, stop the Coder source
at the final writer freeze, and validate its own Pod -> PVC -> PV -> Longhorn chain.

The current product exposes normal managed creation through `POST /api/v1/workspaces` and lifecycle
actions through `POST /api/v1/workspaces/{workspace_id}/actions/{action}`. Use the following
standard, single-workspace procedure; it does not require a special Coder-import API or a database
edit.

1. Before creating the target, read-only verify the Coder source's actual home path, numeric
   UID/GID, file ownership and Pod -> PVC -> PV chain. Select a target template whose
   `workspace_user`/`workspace_home` resolve to the compatible user and home mount. The bootstrap
   does not recursively rewrite durable ownership. Preserve `.codex/sessions`, logs, SQLite/WAL,
   auth/configuration and repositories; only explicitly disposable scratch/cache paths may differ.
2. Create one normal managed target using the authorized organization/owner/template request and
   idempotency key. It receives a new workspace ID and deterministic PVC name. Wait for normal
   provisioning to create and bind its initially empty target PVC/PV, then record its identity,
   labels, claim UID and PV.
3. Call the target workspace `stop` action. Wait until its API state is `Stopped`, StatefulSet
   replicas are observed as zero, no target Pod remains, and its Longhorn volume is detached. A
   stopped reconcile keeps replicas zero and does not overwrite the PVC binding.
4. At the final source freeze, stop every Coder writer from the external operator context, take and
   verify a fresh source snapshot, and wait for the Coder Pod and its Longhorn attachment to be
   gone. Do not perform this step from the Coder workspace itself.
5. Change both the target empty PV and the Coder source PV to `Retain` and read the policies back.
   Keep the target empty PV: it is rollback data and must not be deleted. Delete only the stopped
   target PVC and the frozen source Coder PVC after their `Retain` readback; never delete either PV
   or Longhorn volume. Clear the source PV `claimRef` under the approved change record.
6. Create the replacement target PVC with the **same deterministic target name**, source-PV
   `spec.volumeName`, compatible class/access/volume-mode/capacity, and the ownership labels copied
   from the original target PVC. Wait for Bound, then verify the source PV `claimRef` has the target
   namespace/name and the replacement target PVC's **new UID**. Do not start while the target PVC
   is absent: the StatefulSet claim template would otherwise provision a replacement claim.
7. Re-read Longhorn health/attachment and the PVC/PV chain, then call target `start`. Validate host
   key continuity where applicable, SSH, Web Shell, durable home/Codex data and a single writer.
   Keep both retained PVs and snapshots through the Coder rollback window. This phase may not pause
   or stop the four accepted MWC workspaces.

## Cutover acceptance and rollback

The MWC platform phase is accepted only when: the target namespace exists; it has exactly one
control plane and four MWC workspace identities; all four MWC PV claimRefs point to target claims
with matching UIDs; each MWC Longhorn volume is healthy and has exactly one expected attachment;
schema 22 is confirmed after the published 19->20 bridge and offline 20->22 migration; GitOps/Argo
desired state is target-only; no old MWC controller can reconcile; and each MWC workspace passes
its connectivity/data checks without durable `.codex` loss. The final global acceptance additionally
requires the external Coder phase and its separately recorded acceptance.

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
snapshots. Release production schema 19->20 using the published bridge and validate it. Then stop
the old coordinator and use the current verified-and-published schema-22 release for offline
20->22 migration; do not start an intermediate schema-21 process. Export/import only at matching
schema versions into an empty import target. CI/provenance or publication not green means no
cutover; read exact image digests from successful CI provenance rather than guessing. In this first
phase, stop only the four MWC writers, confirm their detachment, set their selected PVs to Retain,
and rebind them one at a time with matching old/new claim UID checks. Never mount old and target
claims simultaneously. Move/control SQLite only via supported encrypted export/import while
offline (or a storage-owner-approved local-path move). Preserve all .codex except explicitly
identified disposable tmp caches. Leave Coder running.

Validate Pod/PVC/PV identity, Longhorn attachments, schema, SSH host keys, SSH, Web Shell, routes,
and durable data for the four MWC workspaces. Keep old MWC namespaces/controllers/PVs for the
agreed rollback window, then make MWC GitOps target-only. In the later external Coder change,
retain normal four-MWC service and use the documented managed create -> stop -> Retain -> same-name
PVC rebind -> start procedure. Only after Coder acceptance and both rollback windows may global
cleanup prove no old namespace resources remain. Report commands, redacted metadata evidence,
snapshots, CI digest provenance, downtime interval, acceptance results, and any blocked gate.
Never use a broad delete-namespace command.
```
