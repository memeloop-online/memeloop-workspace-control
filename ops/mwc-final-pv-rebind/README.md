# Four-MWC PV direct-rebind handoff

This directory is an **operator handoff only**.  Nothing here is applied by
GitOps or a controller.  It deliberately names four MWC workspace volumes
only; the independent `coder` namespace and its 100Gi Coder home PV are out
of scope.

The target namespace is `memeloop-workspace-control`.  The target namespace
must exist before the target PVCs are created.  Its absence before that point
is an expected precondition, not a failure.

## Immutable source-to-target map

| Workspace | Source namespace | PVC name (preserved) | Source PVC UID | PV | Capacity |
| --- | --- | --- | --- | --- | --- |
| maintainance | `ws-k3si-7032544955-bd2dc9ca6aa2b1b5` | `workspace-data-w-bd2dc9ca6aa2b1b5-0` | `611ec96e-4bd9-4162-b237-dab2b68d3694` | `pvc-611ec96e-4bd9-4162-b237-dab2b68d3694` | 2Gi |
| tiddlywiki-dev | `ws-k3si-7032544955-b268ff46894a14b9` | `workspace-data-w-b268ff46894a14b9-0` | `ae9f3ccf-458d-4f3a-a1a2-97c5e2f84c96` | `pvc-ae9f3ccf-458d-4f3a-a1a2-97c5e2f84c96` | 30Gi |
| game-forking | `ws-k3si-7032544955-97645a0fb4771b1d` | `workspace-data-w-97645a0fb4771b1d-0` | `bf5e159e-bf80-47ae-bda3-58affce88008` | `pvc-bf5e159e-bf80-47ae-bda3-58affce88008` | 60Gi |
| rust-dev-test | `ws-k3si-7032544955-b405d2441a1ee149` | `workspace-data-w-b405d2441a1ee149-0` | `60f341a2-37fb-4d76-80c4-bc657ab0a44a` | `pvc-fe7130d7-18a2-470e-824e-a0dea84e9a5e` | 100Gi |

All four use `mwc-longhorn-large-delete`, `ReadWriteOnce`, and `Filesystem`.
The PVC labels in the individual `pvc-*.yaml` files preserve the workspace, owner, and
managed-by identity labels from each source PVC.  Do not add an owner
reference or alter the workspace IDs.

## Required gates

`control-schema22-job.yaml` is an offline-only database job, not a deployment.
Apply it only after the repaired schema-20 bridge has passed, the schema-20 backup is
verified, and the old control-plane StatefulSet is zero with no remaining writer Pod.
It runs the already published schema-22 binary against the source control claim and
does not launch an API server or coordinator. Require Job completion and schema-22
metadata verification, then remove its completed Pod before rebinding the control PV.
Never apply the entire directory with `kubectl apply -f`: these are ordered, individually
gated artifacts, not a single deployable bundle.

1. The old MWC coordinators must remain stopped/frozen and the four source
   StatefulSets must remain at zero replicas.  Check there is no source
   workload Pod or Longhorn attachment.  This avoids a second writer while a
   PV claim is moved.
2. The approved database offline migration/import and corresponding release
   gate must be complete before the target controller is allowed to reconcile
   these workspaces.  Do not start a schema-21 coordinator as an intermediate
   writer.
3. Confirm the named fresh snapshot for each named PV is `readyToUse=true`.
   The snapshots are rollback evidence; do not delete them in this procedure.
4. Work **one row at a time**, and do not proceed to the next row until the
   target PVC is Bound to its expected PV.  There is no `kubectl delete
   namespace`, selector delete, PV delete, or Coder operation in this plan.

## Per-row procedure

The committed Retain patches contain the source PVC UID and the PV
`resourceVersion` observed when this handoff was made.  A JSON Patch `test`
failure is a deliberate stop signal: re-read that single PV and reconcile the
unexpected change; do not remove the tests.  Immediately before each patch,
check the old claim still matches the row:

```bash
kubectl get pv <PV> -o jsonpath='{.metadata.resourceVersion}{"\\n"}{.spec.persistentVolumeReclaimPolicy}{"\\n"}{.spec.claimRef.namespace}{"/"}{.spec.claimRef.name}{" uid="}{.spec.claimRef.uid}{"\\n"}'
```

1. Apply the matching `retain-*.json` with `kubectl patch pv <PV> --type=json
   --patch-file ops/mwc-final-pv-rebind/retain-*.json`; then read back and
   require `Retain` plus the exact old claim UID.  The patch tests the old
   claim namespace/name as well as UID and `resourceVersion`.
2. Delete only that exact old PVC, never its PV. Wait until that PVC is absent
   and the retained PV reports `Released`. Its old `claimRef` normally remains;
   do not wait for Kubernetes to clear it automatically. Re-read the PV.
3. Construct a JSON Patch from `clear-claimref.template.json` for this one PV:
   substitute the newly read PV `resourceVersion` (after Retain), its exact
   old namespace/name/UID, and its PV name.  Apply it only after all `test`
   operations pass.  This clears the stale source `claimRef`; it does not
   recreate or format a Longhorn volume.
4. Apply only the matching `pvc-*.yaml` file in
   `memeloop-workspace-control`.  Its `spec.volumeName` is the existing PV.
   It must not be dynamically provisioned to a different PV.
5. Record the newly allocated target PVC UID and prove both directions before
   allowing a workspace to start:

```bash
kubectl get pvc -n memeloop-workspace-control <PVC> -o jsonpath='{.metadata.uid}{"\\n"}{.spec.volumeName}{"\\n"}{.status.phase}{"\\n"}'
kubectl get pv <PV> -o jsonpath='{.spec.claimRef.namespace}{"/"}{.spec.claimRef.name}{" uid="}{.spec.claimRef.uid}{"\\n"}{.status.phase}{"\\n"}'
```

The result must be `Bound` in both commands, have the canonical namespace and
preserved PVC name, have `<PV>` in `pvc.spec.volumeName`, and have the **new
target PVC UID** in `pv.spec.claimRef.uid`.  The old source PVC UID is only a
pre-move guard and must not be compared against the new target UID.

Keep the reclaim policy `Retain` throughout migration, validation, and the
rollback window.  Only after acceptance may an explicitly approved operator
restore the StorageClass-appropriate policy (currently `Delete`).

## Stop / rollback boundary

Stop immediately if a patch test fails, a source writer/attachment reappears,
the PV binds to another claim, or a target PVC is not Bound to the expected
PV.  Do not start the target workspace.  Retain preserves the PV and the
verified snapshot permits the established same-schema recovery path.  No
namespace-wide cleanup is permitted until all five workspaces, including the
separate later Coder handoff, have completed their own acceptance windows.
