# gVisor sandbox node operations

This is the short runbook for enabling the optional `runsc` handler on one K3s node. It keeps
`runc` as the default and does not enable production gVisor workloads until the canary passes.

## Current state: `serv-146231`

- Target identity is Kubernetes node `serv-146231`, host `serv.146231.com`, address
  `100.64.0.10`. The user authorized its AlmaLinux 8 to 9 upgrade and necessary reboots; no
  node-data backup is required. Follow [the OS upgrade plan](SERV-146231-OS-UPGRADE-PLAN.md)
  for the current maintenance/recovery state instead of recording node-event history here.
- AlmaLinux 9.8 has booted with kernel `5.14.0-687.42.1.el9_8` and systemd 252.
  K3s automatic startup is restored and the node is Ready and schedulable.
  The Wiki volume is attached and healthy; Wiki and overseas Higress have recovered.
- The optional `runsc` handler is installed; runc remains the default. A temporary
  `gvisor-canary` passed the basic non-root, filesystem, process, DNS and loopback checks.
  Its host Pod cgroup was configured for 1024 host tasks, 0.5 CPU and 512 MiB.
  API workspace startup, SSH/Web Shell, persistent Home, scratch cleanup and
  bounded resource checks are recorded in
  [workspace acceptance](GVISOR-WORKSPACE-ACCEPTANCE.md).
  The node is registered with `sandbox.memeloop.dev/gvisor-ready=true`;
  the formal `gvisor` RuntimeClass is managed by GitOps. Do not reinstall the
  handler. External-tenant network isolation is still a separate pending gate.
- The runsc handler uses `overlay2 = "root:memory,size=128m"`. A disposable
  container reached the 128 MiB root-filesystem limit, received `ENOSPC`, and
  continued executing commands; deleting its file restored writes. New sandboxes
  picked up this configuration without a K3s restart. Mounted Home and scratch
  volumes are separate and are not covered by this root-filesystem cap.

Use `--rootfs-memory-mib 128` when installing this sandbox configuration on a
new node. Keep the root overlay setting when upgrading runsc. The exact
node configuration and reversal instructions are in GitOps under
`apps/memeloop-workspace-control/node-configuration/`.

The cgroup decision is fixed for this K3s setup: cgroup v2 and K3s' `SystemdCgroup=true` send
the CRI path as a systemd slice such as
`kubepods-burstable-pod<UID>.slice:cri-containerd:<sandbox-id>`. gVisor's default filesystem
driver does not interpret that slice path. After AlmaLinux 9 is ready, use gVisor's systemd
driver only when systemd is >=244; do not switch kubelet/runc to cgroupfs as a workaround.

## Install the handler on another eligible node

1. On the exact target node, run the read-only preflight from the reviewed scripts directory:

   ```bash
   sudo ./gvisor-node-preflight.sh
   ```

   It must report the target's active K3s service, Linux kernel >=5.6, containerd 2.x with the
   K3s v3 template, cgroup v2 with systemd >=244, and `runc` still as the default. Resolve a
   failed check before installing anything.
2. Download a current pinned official gVisor release and verify its published SHA-256 at the
   time of installation. The archive must contain `runsc`, `containerd-shim-runsc-v1` and the
   adjacent `gvisor-bin/` sidecars. Do not copy a historical digest into this document.
3. Install only on the target node and explicitly request the one K3s restart:

   ```bash
   sudo ./gvisor-node-install.sh \
     --node "$(hostname)" --archive /path/to/gvisor-archive.tar.* \
     --sha256 '<current-official-release-sha256>' --apply --restart-k3s
   ```

   The installer stores versioned binaries, extends
   `/var/lib/rancher/k3s/agent/etc/containerd/config-v3.toml.tmpl`, and leaves `runc` as the
   default. On cgroup v2 it configures runsc with `systemd-cgroup = "true"`; it also keeps a
   timestamped template copy for handler rollback. The restart affects Pods and image
   operations on this node, so keep the node in its OS maintenance state during the restart.

## RuntimeClass and canary

Configure an explicit kubelet `podPidsLimit` during node maintenance. This node uses
1024 in `/var/lib/rancher/k3s/agent/etc/kubelet.conf.d/90-mwc-pids.conf`.
The installation and rollback template lives in GitOps under
`apps/memeloop-workspace-control/node-configuration/`. Preserve a stricter existing
limit. After restarting K3s, verify kubelet `configz` and the new canary's actual
host cgroup `pids.max`. This limits host tasks, not the number of guest processes:
the live gVisor workspace spawned 1100 short-lived guest children with a host
limit of 1024 and no host PID-limit hits. Do not advertise this setting as a
guest process quota. Retain host CPU/memory limits and verify recovery under
bounded guest-process pressure.

After the handler restart, verify the K3s service, rendered containerd configuration, CRI
plugin, and node readiness. Create a temporary `gvisor-canary` RuntimeClass with handler
`runsc` and scheduling selector `kubernetes.io/hostname: serv-146231`. Use it only for a
disposable, non-privileged canary exercising the workspace network, volume, process, signal
and observability paths. If the node remains cordoned, give only that canary the
`node.kubernetes.io/unschedulable:NoSchedule` toleration. Do not begin with privileged,
`hostNetwork`, GPU, host-device or hostPath workloads.

The RuntimeClass shape is:

```yaml
apiVersion: node.k8s.io/v1
kind: RuntimeClass
metadata:
  name: gvisor
handler: runsc
scheduling:
  nodeSelector:
    sandbox.memeloop.dev/gvisor-ready: "true"
```

Create this production RuntimeClass and add `sandbox.memeloop.dev/gvisor-ready=true` only after the
handler and canary pass on this exact node. Compare the canary with the normal runtime and
measure CPU, memory, PID, ephemeral-storage and startup overhead before declaring a production
overhead value. Remove the temporary canary and its RuntimeClass when testing ends.
A passing handler check alone is not RuntimeClass acceptance.

## Rollback

First remove or stop gVisor workloads and wait until none use `runsc`; remove the readiness
label and do not delete a RuntimeClass while workloads still reference it. Restore the
timestamped containerd template copy under
`/var/lib/rancher/k3s/agent/etc/containerd/gvisor-backups/`, remove the handler links and
versioned binaries after confirming they are unused, restart only this node's K3s service, and
verify the node returns Ready with `runc` as the default.

If no template existed before installation, there is no old template backup. Remove only
the added `runsc` stanzas from the new template, preserving the base template and any
unrelated changes. This is the case on `serv-146231`.

If the AlmaLinux upgrade itself cannot be repaired, use the OS plan's rebuild/rejoin path. The
approved OS event has no backup-based rollback promise; this document does not add one.

References: [K3s advanced containerd configuration](https://docs.k3s.io/advanced),
[gVisor installation](https://gvisor.dev/docs/user_guide/install/),
[gVisor containerd configuration](https://gvisor.dev/docs/user_guide/containerd/configuration/),
and [Kubernetes RuntimeClass](https://kubernetes.io/docs/concepts/containers/runtime-class/).
