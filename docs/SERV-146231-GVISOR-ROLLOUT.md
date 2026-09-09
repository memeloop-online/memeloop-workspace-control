# serv-146231 gVisor rollout status

This document records the current target state and the next usable gVisor steps. It is not a
completion claim.

## Current state

- Target: Kubernetes node `serv-146231`, host `serv.146231.com`, `100.64.0.10`.
- The user authorized the AlmaLinux 8 to 9 upgrade and its necessary reboots. No backup is
  required for this node; OS execution and recovery are described in
  [the OS upgrade plan](SERV-146231-OS-UPGRADE-PLAN.md).
- ELRepo `5.15.220-1.el8.elrepo` is installed and has booted successfully. Today's EL8 update
  completed; the stock 4.18 boot then hit a K3s/etcd TLS timeout, and a one-shot 5.15 boot
  restored K3s readiness. Follow the OS plan for the temporary K3s shutdown needed to run
  Leapp on the official kernel.
- AlmaLinux 9 is not installed. `runsc` and a gVisor `RuntimeClass` are not installed, so the
  node is not gVisor-accepted and no completion status may be inferred.

## Compatibility decision

The current system is AlmaLinux 8 with systemd 239 and cgroup v2. gVisor's systemd cgroup
driver requires systemd >=244, so the current OS cannot use that driver.

K3s uses `SystemdCgroup=true` and supplies paths such as
`kubepods-burstable-pod<UID>.slice:cri-containerd:<sandbox-id>`. The gVisor filesystem cgroup
driver treats that non-absolute value as a filesystem path and does not map it to the Pod's
systemd hierarchy. The gVisor systemd driver understands the slice notation, but rejects
systemd versions below 244. Therefore:

- Finish the OS upgrade first and verify systemd >=244 and cgroup v2.
- Keep K3s/kubelet/runc on their current systemd cgroup path; do not switch to cgroupfs as a
  workaround.
- Configure runsc with `systemd-cgroup = "true"` only after the post-upgrade preflight passes.

## Next execution

After the OS plan reports AlmaLinux 9 and a healthy K3s/etcd node:

1. Run `gvisor-node-preflight.sh` on `serv-146231`. It must pass the kernel, active K3s,
   containerd-v3 template, cgroup v2 and systemd >=244 checks while retaining `runc` as the
   default.
2. Use a current pinned official gVisor archive, verify its published SHA-256, and confirm it
   contains `runsc`, `containerd-shim-runsc-v1` and `gvisor-bin/` sidecars. Do not use a
   historical digest embedded in this record.
3. Run `gvisor-node-install.sh --node "$(hostname)" --archive <archive> --sha256 <sha256>
   --apply --restart-k3s`. The installer registers the runsc handler in the K3s v3 template,
   retains runc as default, configures the systemd driver for cgroup v2, and checks the CRI
   plugin after the requested restart.
4. Verify the K3s service, rendered handler, CRI plugin, node readiness and ordinary workloads.
   Then create a temporary `gvisor-canary` RuntimeClass restricted to this node and run a
   disposable non-privileged canary with that class. Exercise networking, volumes, processes, signals and
   observability, and compare with runc.
5. Only after the canary passes, add `sandbox.memeloop.dev/gvisor-ready=true` and create/use
   the `gvisor` RuntimeClass. Measure CPU, memory, PID, ephemeral-storage and startup overhead
   before setting a production `RuntimeClass.overhead` value.

The full command and manifest sequence is in
[SANDBOX-NODE-OPERATIONS.md](SANDBOX-NODE-OPERATIONS.md). Do not treat installation or a
passing handler check as RuntimeClass acceptance.

## Rollback

For a gVisor-only failure, remove/stop gVisor workloads, remove the node label, restore the
installer's timestamped `config-v3.toml.tmpl` copy from
`/var/lib/rancher/k3s/agent/etc/containerd/gvisor-backups/`, remove unused runsc links/binaries,
restart this node's K3s service, and verify Ready with runc as default. Keep the RuntimeClass
while any workload references it; remove it only after those references are gone.

For an OS-upgrade failure, follow the OS plan's console repair or AlmaLinux 9 rebuild/rejoin
path. The approved event has no backup-based rollback promise.

References: [gVisor systemd cgroup requirements](https://gvisor.dev/docs/user_guide/systemd/),
[gVisor containerd configuration](https://gvisor.dev/docs/user_guide/containerd/configuration/),
[K3s advanced containerd configuration](https://docs.k3s.io/advanced), and
[Kubernetes RuntimeClass](https://kubernetes.io/docs/concepts/containers/runtime-class/).
