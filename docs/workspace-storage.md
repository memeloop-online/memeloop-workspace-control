# Workspace storage and pressure policy

## Storage layers and quotas

MWC separates durable user data from data that can be regenerated. A template declares one total
temporary-storage capacity through `spec.storage_policy.temporary_storage_gib`. When the platform
configures a scratch StorageClass, that value becomes the request for one Pod-owned generic
ephemeral PVC named `workspace-scratch`. The selected class should provide hard capacity
enforcement; the durable Home class remains a separate choice. Kubelet may still evict a Pod
because of writable-layer or log pressure, so containers keep small local `ephemeral-storage`
requests and limits that do not include PVC capacity.

| Layer | Default | Contents |
| --- | --- | --- |
| Durable Home | Template disk size on a Longhorn PVC | Repositories, user configuration, credentials, Codex conversations and SQLite state |
| Platform connection runtime | 128 MiB memory `emptyDir` | sshd configuration, host-key copy, current authorized keys, kubeconfig, socket/PID files and pressure banner |
| Interactive temporary space | 512 MiB memory `emptyDir` for the workspace and a separate 128 MiB memory `emptyDir` for ttyd | `/tmp` and `/var/tmp`; ttyd cannot exhaust the workspace shell's temporary space |
| Regenerable temporary storage | One template-sized generic ephemeral PVC when the scratch class is set; one bounded disk `emptyDir` otherwise | Workspace/compiler caches, optional BuildKit cache, and Codex/session scratch in isolated subpaths with one shared total quota |

The Home emergency reserve is a platform rule, not a template knob: it is the smaller of 1 GiB and
10% of the Home PVC. The platform-wide pressure thresholds are fixed at 80% and 90%, matching the
runtime API and Prometheus rules. New templates request 512 MiB and limit 2 GiB of local
`ephemeral-storage` for the workspace container's writable layer and logs. When enabled, BuildKit
requests 256 MiB and limits 1 GiB for the same purpose. Temporary-storage capacity never inflates
those container values.

### Temporary-storage backing and layout

With Helm `workspace.scratchStorageClassName` configured, the Pod spec contains one Kubernetes
generic ephemeral volume. Its inline claim template requests `ReadWriteOnce` capacity equal to
`temporary_storage_gib`. Kubernetes creates one PVC, makes the Pod its owner, and deletes it with
the Pod. Workspace cache, BuildKit cache, and Codex/session scratch share this hard total capacity;
one consumer filling the volume reduces the capacity available to the others.

The scratch class is intentionally independent from `workspace.storageClassName`. Use a class for
node-local NVMe or another local block/LVM/ZFS pool that enforces requested capacity and uses
topology-aware scheduling and `volumeBindingMode: WaitForFirstConsumer`, for example a correctly
configured TopoLVM or local LVM/ZFS CSI class. Do not default scratch to replicated Longhorn Home
storage: regeneration does not justify its network and replica overhead. If no scratch class is
configured, MWC renders the same single volume as a disk-backed `emptyDir` with `sizeLimit` equal to
the template capacity. This compatibility fallback is a kubelet-enforced bound, not provisioned
capacity or a scheduler reservation, and remains subject to node ephemeral-storage pressure and
eviction.

The scratch initialization container mounts the whole volume first, rejects unsafe non-directory
or symlink subpaths, and creates only the required top-level directories with exact ownership and
modes. Later containers mount `workspace-cache`, `codex-session-scratch`, and, when BuildKit is
enabled, `build-cache` through Kubernetes `subPath` mounts. A BuildKit-disabled Pod neither creates
the `build-cache` directory nor mounts a BuildKit path. The platform's `/tmp`, SSH runtime, and ttyd
temporary space remain separate small tmpfs volumes and do not consume the scratch quota.

For a new or cleaned Home, MWC links regenerable cache paths into build scratch. A non-empty
cache is never deleted or replaced automatically. Stop the workspace, clean that cache
explicitly, and start it again; the empty path is then linked to the bounded layer.

## Lifecycle and cleanup

- A running Pod owns all scratch data, either through one generic ephemeral PVC owner reference or
  through one `emptyDir`. No job deletes files by age and no mtime policy can race an active compiler
  or linker.
- Stop scales the StatefulSet to zero. Kubernetes removes the Pod and automatically reclaims its
  generic ephemeral scratch PVC, plus temporary and connection-runtime `emptyDir` volumes; the Home
  PVC remains.
- Start creates clean Pod-lifetime volumes and re-materializes current keys, kubeconfig, and
  template-selected credential and file injections. Restart has the same scratch cleanup semantics.
- Delete removes the workspace Namespace, Home PVC, Secrets, ConfigMaps, routes, and runtime data
  through the normal ownership-checked deletion flow.

## Pressure and connection continuity

The runtime API reports `storage.used_percent` and `storage.pressure`. The Helm
`PrometheusRule` records `mwc_workspace_home_used_percent` and raises warning and critical alerts:

- Below 80%: normal operation. A platform-owned Home reserve is allocated when `fallocate` is
  available.
- At 80%: the API reports `warning`, Prometheus alerts after the configured duration, and new SSH
  sessions show a cleanup banner. Build and temporary writes already use bounded scratch, so no
  destructive cleanup is started.
- At 90%: the API reports `critical`, the critical alert fires, and MWC releases only its own Home
  reserve once to give SQLite and the user room to finish and clean up. Durable file
  materialization may enter a visible degraded state instead of blocking sshd.
- At 100% on an existing Home: runtime directories, authorized keys, kubeconfig, sshd and ttyd do
  not require a Home write. Optional durable-directory/cache-link updates are best-effort and mark
  the runtime degraded. A missing, read-only, or otherwise invalid Home mount remains a hard error.

The same rule group records MWC's own local ephemeral-storage requests for writable-layer and log
capacity planning. On every node currently carrying a workspace from this installation it also
records all Pods' combined ephemeral-storage request percentage, alerts at 80/90%, and alerts when
Kubernetes reports
`DiskPressure`. Joining through `kube_pod_info` works whether or not the installed
kube-state-metrics version adds a `node` label directly to resource-request series. Scoping the
rules to this installation's active workspace nodes prevents unrelated-node alerts and labels each
alert with `installation_id`. These node alerts cover writable-layer, log, and
compatibility-fallback `emptyDir` eviction risk. Scratch PVC pool capacity must be monitored
through the selected CSI driver's storage metrics.

MWC intentionally has no workspace agent. Native SSH is standard OpenSSH, and Web Shell is
browser → Higress → ttyd → localhost OpenSSH. BuildKit is a regular sidecar and cannot gate
sshd readiness. Workspace Services continue publishing the Pod endpoint for this recovery channel
even while an optional sidecar reports unready.

## Codex state and logs

The whole `.codex` directory is never placed on an ephemeral volume. Conversation/session data,
logs, SQLite state and its WAL/SHM files stay on Home; only `.codex/tmp` and `.codex/.tmp` are
regenerable. MWC never runs a sidecar, scheduled cleanup, online `VACUUM`, or any other process
that concurrently edits Codex SQLite files.

Current Codex releases retain log rows for ten days and bound each thread/process log stream to
approximately 10 MiB or 1,000 rows. Startup performs a passive checkpoint, not a `VACUUM`, so
upgrading limits future retained rows but does not necessarily reduce already allocated
`logs_2.sqlite` file size. Reclaiming that allocation is an offline maintenance operation only:

1. fully stop the workspace and confirm no Codex process has the database open;
2. snapshot or back up the Home PVC;
3. run SQLite integrity checks against the stopped copy;
4. compact it offline and verify integrity again before starting the workspace.

The platform does not automate this sequence because an interrupted or concurrent compaction can
lose threads or corrupt state.

## Existing infrastructure

- Longhorn provides durable Home volumes, snapshots, and offline recovery points.
- A separately selected local CSI StorageClass provides temporary-storage capacity; one Kubernetes
  generic ephemeral volume binds its PVC lifecycle to the workspace Pod.
- Kubernetes enforces Pod lifecycle and bounds memory or compatibility-fallback `emptyDir` volumes.
- Existing kubelet PVC metrics feed Prometheus; Prometheus Operator installs the recording and
  alert rules; Grafana and Alertmanager visualize and route them.
- OpenSSH, ttyd, BuildKit, and control-plane stdout/stderr remain ordinary Kubernetes container
  logs and can be collected by the cluster's existing Loki pipeline.

MWC uses Kubernetes, Longhorn, Prometheus/Grafana/Alertmanager/Loki, Higress, standard OpenSSH,
and ttyd.

Kubelet PVC series identify the workspace Namespace. Grafana can attach workspace, organization,
and owner dimensions from kube-state-metrics without adding unbounded labels to MWC metrics:

```promql
mwc_workspace_home_used_percent
  * on (namespace) group_left (
      label_workspace_memeloop_dev_workspace_id,
      label_workspace_memeloop_dev_organization_id,
      label_workspace_memeloop_dev_owner_user_id
    ) kube_namespace_labels
```
