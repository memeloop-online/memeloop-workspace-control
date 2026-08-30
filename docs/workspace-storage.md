# Workspace storage and pressure policy

## Storage layers and quotas

MWC separates durable user data from data that can be regenerated. `emptyDir.sizeLimit` values are
upper bounds, not node-disk reservations. Kubelet may still evict a Pod under node-wide
ephemeral-storage pressure; template requests provide scheduler capacity signals, while limits and
volume bounds contain a single workspace.

| Layer | Default | Contents |
| --- | --- | --- |
| Durable Home | Template disk size on a Longhorn PVC | Repositories, user configuration, credentials, Codex conversations and SQLite state |
| Platform connection runtime | 128 MiB memory `emptyDir` | sshd configuration, host-key copy, current authorized keys, kubeconfig, socket/PID files and pressure banner |
| Interactive temporary space | 512 MiB memory `emptyDir` for the workspace and a separate 128 MiB memory `emptyDir` for ttyd | `/tmp` and `/var/tmp`; ttyd cannot exhaust the workspace shell's temporary space |
| Regenerable build data | 12 GiB node-local `emptyDir` | compiler `TMPDIR`, Cargo target output, `$HOME/.cache` for new/clean Homes, and package-manager caches |
| Rootless image builds | 8 GiB node-local `emptyDir` | BuildKit state, cache, configuration, socket, temporary files and `buildctl` |
| Codex scratch | 2 GiB node-local `emptyDir` | Only `$HOME/.codex/.tmp`; the rest of `.codex` remains durable |

Templates may override volume sizes. The Home emergency reserve is selected automatically as
the smaller of 1 GiB and 10% of the PVC; templates may choose a smaller explicit value through
`spec.storage_policy`. The platform-wide pressure thresholds are deliberately fixed at 80% and
90%, matching the runtime API and Prometheus rules. MWC raises effective container
ephemeral-storage limits to cover the build and Codex scratch boundaries plus runtime headroom.
This limit is containment, not reservation. New templates request 2 GiB for the workspace
container, editable in the template form or YAML source. When enabled, BuildKit requests 1 GiB
separately and its limit follows the configured BuildKit cache boundary.

For a new or cleaned Home, MWC links regenerable cache paths into build scratch. A migrated
non-empty cache is never deleted or replaced automatically. Stop the workspace, clean that cache
explicitly, and start it again; the empty path is then linked to the bounded layer.

## Lifecycle and cleanup

- A running Pod owns all `emptyDir` data. No job deletes files by age and no mtime policy can race
  an active compiler or linker.
- Stop scales the StatefulSet to zero. Kubernetes removes the Pod and all build, temporary,
  BuildKit, Codex scratch, and connection-runtime volumes; the Home PVC remains.
- Start creates clean Pod-lifetime volumes and re-materializes current keys, kubeconfig, files, and
  environment declarations. Restart has the same scratch cleanup semantics.
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

The same rule group records MWC's own ephemeral-storage requests for capacity planning. On every
node currently carrying a workspace from this installation it also records all Pods' combined
ephemeral-storage request percentage, alerts at 80/90%, and alerts when Kubernetes reports
`DiskPressure`. Joining through `kube_pod_info` works whether or not the installed
kube-state-metrics version adds a `node` label directly to resource-request series. Scoping the
rules to this installation's active workspace nodes prevents unrelated-node alerts and labels each
alert with `installation_id`. These node alerts cover eviction risk that an individual
`emptyDir.sizeLimit` cannot prevent.

MWC intentionally has no workspace agent. Connectivity therefore does not share the failure mode
of a Home-backed Coder agent: native SSH is standard OpenSSH, and Web Shell is
browser → Higress → ttyd → localhost OpenSSH. BuildKit is a regular sidecar and cannot gate sshd
startup if its own bounded volume fails. Workspace Services continue publishing the Pod endpoint
for this recovery channel even while an optional sidecar reports unready.

## Codex state and logs

The whole `.codex` directory is never placed on an ephemeral volume. Conversation/session data and
SQLite state stay on Home; only `.codex/.tmp` is regenerable. MWC never runs a sidecar, scheduled
cleanup, online `VACUUM`, or any other process that concurrently edits Codex SQLite files.

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
- Kubernetes enforces Pod lifecycle and bounded `emptyDir` volumes.
- Existing kubelet PVC metrics feed Prometheus; Prometheus Operator installs the recording and
  alert rules; Grafana and Alertmanager visualize and route them.
- OpenSSH, ttyd, BuildKit, and control-plane stdout/stderr remain ordinary Kubernetes container
  logs and can be collected by the cluster's existing Loki pipeline.

No Tailscale or Coder Premium capability is required. MWC uses only Kubernetes, Longhorn,
Prometheus/Grafana/Alertmanager/Loki, Higress, standard OpenSSH, and ttyd.

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
