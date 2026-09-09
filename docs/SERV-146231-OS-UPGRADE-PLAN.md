# serv-146231 AlmaLinux 8 to 9 upgrade plan

The OS upgrade is complete: AlmaLinux 9.8, kernel `5.14.0-687.42.1.el9_8`,
systemd 252, K3s enabled and active, and Node Ready. Do not repeat ELevate.
The node remains in maintenance for gVisor acceptance; restore Wiki scheduling
and volume attachment when that maintenance ends.

The user approved the AlmaLinux 8 to 9 upgrade and the required reboots for
`100.64.0.10` (`serv-146231`, `serv.146231.com`). This is a single-node control-plane
maintenance event. It covers the OS and the checks needed to return this node to service;
it does not include a K3s, Longhorn, CNI or gVisor upgrade.

The node is accepted as having no important local data. No cloud snapshot, off-node backup,
clone rehearsal or multi-operator sign-off is a prerequisite. Consequently this plan has no
guaranteed rollback: if the upgraded node cannot be repaired, rebuild it as AlmaLinux 9 and
rejoin it using the supported K3s procedure.

## Before the maintenance window

1. Confirm that the target is `serv-146231` / `100.64.0.10`, and confirm access to the
   provider or serial console and the ability to reset the node. SSH, Tailscale and GRUB are
   useful but are not the only recovery path.
2. Confirm that the other K3s servers are `Ready` and that etcd has quorum without this node.
   Do not start the upgrade if taking this server offline would lose quorum.
3. Mark the node unschedulable. Drain only workloads that actually need eviction, using the
   existing Kubernetes/Longhorn process; do not change replica counts or delete replicas.
4. Check this node's Longhorn volumes and iSCSI state. Its single-replica wiki volume will
   be unavailable during maintenance; this outage is accepted. Do not delete its replica
   or block this change on unrelated degraded volumes elsewhere in the cluster.
5. Check free space in `/`, `/boot`, `/var` and the K3s data path. Record the currently running
   kernel and the enabled AlmaLinux, K3s, Tailscale and ELRepo repositories. Keep unrelated
   package, kernel, K3s and storage changes out of this window.

## Leapp assessment and execution

The updated EL8 4.18 kernel boots the operating system, but this node's K3s/etcd
does not recover under it. K3s recovers under the installed ELRepo 5.15 kernel;
Leapp cannot identify that kernel's distribution package and refuses to proceed.
Use the official EL8 kernel for Leapp while keeping this node's K3s temporarily
stopped and disabled. Keep the node cordoned and the other control-plane members
healthy. This is planned single-node downtime, not permission to change etcd data
or bypass Leapp's kernel check. Restore K3s's original enabled state and start it
after EL9 boots. If assessment prevents starting the upgrade transaction,
restore the known working 5.15 boot and K3s instead. Once EL9 userspace changes
begin, use the recovery procedure below rather than treating an EL8 kernel as rollback.

1. Fully update AlmaLinux 8 using the current AlmaLinux ELevate procedure, reboot as required
   by that procedure, and verify that the node returns before continuing.
2. Install the current ELevate/Leapp packages from the official procedure and run
   `leapp preupgrade`.
3. Read the current Leapp report and answer file. Resolve every item explicitly labelled
   `Inhibitor`; a warning or informational entry is not an inhibitor by itself. Typical items
   to review are unsupported third-party packages or repositories (including ELRepo kernel,
   K3s and Tailscale), missing EL9 replacements, repository/module conflicts, required
   answer-file values, and insufficient disk space. Do not treat this list as a substitute for
   the report and do not suppress an inhibitor without understanding its effect.
4. Rerun `leapp preupgrade` after each change until the report has no unresolved inhibitors.
   If the report proposes removing or replacing a package needed for K3s, networking, iSCSI,
   cloud-init or the firewall, stop and review that specific change before proceeding.
5. In the approved window, run the official `leapp upgrade` procedure and reboot from the
   console when instructed. Watch the upgrade boot and keep the node unschedulable. Do not
   combine this reboot with a K3s or gVisor change.

## Recovery checks after reboot

Keep the node unschedulable until all of these checks pass:

- Confirm AlmaLinux 9, the expected boot entry, systemd >=244, cgroup v2 and adequate disk
  space.
- Confirm console access, the public route, DNS, SSH, Tailscale, cloud-init and the firewall.
- Confirm iSCSI sessions, Longhorn disks, volumes and replicas, with a controlled application
  I/O check where applicable.
- Confirm the K3s server is active, this node is `Ready`, the remaining control-plane members
  and etcd are healthy, and CoreDNS plus critical workloads recover. Do not upgrade K3s here.
- Only after the control-plane, storage and network checks pass, make the node schedulable
  again and verify normal scheduling.

If a check fails, use the console to repair the node or keep it out of service. If repair is
not practical, rebuild/reprovision an AlmaLinux 9 node and rejoin it through the supported K3s
control-plane procedure. Retained EL8 kernels cannot roll back the EL9 userspace, and there is
no snapshot or backup from which to promise restoration. If the surviving etcd quorum is lost,
cluster recovery requires reconstruction from whatever declarations and credentials remain;
this plan does not claim a datastore rollback.

Stop for a real unresolved Leapp `Inhibitor`, loss of control-plane quorum, or failed
post-reboot network, iSCSI, K3s or etcd checks. Request the user's console assistance
if SSH recovery fails.

## References

- [AlmaLinux ELevate quickstart](https://wiki.almalinux.org/elevate/ELevate-quickstart-guide.html)
  and [migration paths](https://wiki.almalinux.org/elevate/)
- [gVisor systemd cgroup requirements](https://gvisor.dev/docs/user_guide/systemd/)
- [K3s etcd snapshots and restore](https://docs.k3s.io/cli/etcd-snapshot) and
  [manual upgrade behavior](https://docs.k3s.io/upgrades/manual)
- [Longhorn node eviction](https://longhorn.io/docs/1.12.1/nodes-and-volumes/nodes/disks-or-nodes-eviction/)
