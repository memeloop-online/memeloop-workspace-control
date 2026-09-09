# serv-146231 AlmaLinux 8 to 9 upgrade plan

User decision (2026-09-09): executing this node's AlmaLinux 8 to 9 upgrade is approved.
The recovery, datastore and storage checks below still apply before the upgrade transaction.

The authorization covers the OS upgrade and its necessary maintenance steps, not unrelated
K3s, Longhorn or networking upgrades. The target is
control-plane node `serv-146231` (`serv.146231.com`, cloud/Tailscale address `100.64.0.10`), not a
disposable workspace worker.

## Decision and scope

The in-place route to investigate is AlmaLinux 8.10 to AlmaLinux 9 via AlmaLinux ELevate/Leapp's
one-major-version path. AlmaLinux documents AlmaLinux 8 as an eligible EL9 source and requires a
fully updated source system followed by a reboot before migration. `leapp preupgrade` is only a
decision input, not permission to proceed. A newly provisioned AlmaLinux 9 replacement remains
safer because it avoids mutating a live etcd/Longhorn/K3s server in place.

The purpose is to obtain and verify an installed systemd >=244, which gVisor requires when its
optional `--systemd-cgroup` driver is enabled. This plan does not promise a given EL9 image or
completed ELevate transaction meets that requirement: verify systemd, cgroup v2, K3s and runtime
behavior after the upgrade. It does not alter the previous gVisor cgroup-path conclusion, register
a handler, or approve a RuntimeClass.

## Read-only baseline (2026-09-08)

| Area | Observed state | Consequence |
| --- | --- | --- |
| OS | AlmaLinux 8.10, systemd 239, cgroup v2 | Major OS migration is needed for the gVisor systemd-driver prerequisite. |
| Boot | Running ELRepo `5.15.220-1.el8.elrepo`; saved GRUB default is AlmaLinux 4.18; serial console configured | An EL8 one-shot kernel rollback cannot roll back an EL9 userspace transaction. |
| K3s | `v1.36.2+k3s1`, enabled and active | Multi-server control-plane maintenance; do not combine a K3s upgrade/configuration change. |
| Network | public `eth0`, Tailscale `100.64.0.10`, CNI `10.42.5.0/24`, static default route | Test console, public route, Tailscale, DNS and CNI after migration. |
| Storage | `iscsid` active with a Longhorn iSCSI session | Replica, engine and iSCSI health are gates, not follow-up work. |
| Tooling | no ELevate/Leapp package installed | No migration tool has been staged or run. |
| Repos/packages | AlmaLinux, K3s and Tailscale repos enabled; ELRepo kernel packages installed | Preupgrade must explicitly review ELRepo kernel, K3s, Tailscale, iSCSI, cloud-init, CNI and all third-party packages. |

## Execution gates for a separately approved change

1. **Replacement first.** Assess and price a new AlmaLinux 9 node path. Use in-place upgrade only
   after an owner accepts its greater outage and rollback risk.
2. **Console and snapshot.** The user must demonstrate cloud-console login, power reset and a
   restorable VM/disk snapshot. Capture its ID and trial the restore path on a non-production clone
   where feasible. SSH, Tailscale, cloud-init and saved GRUB alone are insufficient recovery.
3. **etcd safety.** The control-plane owner must verify peer readiness and quorum, create a fresh
   on-demand embedded-etcd snapshot, verify an off-node copy, and name the restore operator. Treat
   snapshots and their bootstrap material as restricted; never put tokens or contents in this plan.
4. **Longhorn and applications.** All volumes must be healthy and not rebuilding, including
   previously degraded `pvc-6a03d5cb-b64b-43e1-b236-5bf3de83a613`. Confirm healthy replicas off
   node, current tested backups, and enough eligible disk capacity for any eviction. Do not change
   replica count or delete replicas to satisfy this gate.
5. **Maintenance plan.** Inventory local workloads, PDBs, ingress/DNS capacity and approved
   Kubernetes/Longhorn drain/eviction steps. K3s restart behavior does not itself drain a node;
   the OS event is longer and less predictable.
6. **Clone and package review.** On a matching clone, perform the official preupgrade assessment
   and resolve every inhibitor with its owner. Review every proposed removal/replacement, especially
   ELRepo `kernel-lt`, K3s, Tailscale, iSCSI initiator, cloud-init and network/firewall tooling.
   A green report is necessary, not sufficient.
7. **Capacity and isolation.** Record free space for `/`, `/boot`, `/var` and K3s data. Complete
   the provider snapshot before disk changes. Freeze OS, K3s, Longhorn, CNI, firewall, kernel and
   gVisor changes into separate maintenance events.

## Later execution outline

1. On the matching clone, preserve the official ELevate preupgrade report, transaction list and
   post-upgrade logs. Prove provider-console, network, iSCSI, K3s and Longhorn recovery first.
2. In the approved window, verify every gate, make an independently verified off-node etcd snapshot
   and cloud disk/VM snapshot, and have console plus etcd/Longhorn/network owners present.
3. Evacuate only through the approved Kubernetes/Longhorn process. Stop if required replica health
   cannot be retained or PDBs prevent safe drain.
4. Fully update EL8, perform its required reboot, then rerun `leapp preupgrade`; review every new
   inhibitor, answer and package change. Any unapproved change is a stop condition.
5. Only with explicit approval, run the then-current official AlmaLinux procedure and watch its
   automatic upgrade boot through cloud console. Do not install gVisor or change K3s in this event.
6. Keep the node unschedulable until all acceptance checks pass and control-plane, storage and
   networking owners sign off.

## Acceptance after upgrade

- Verify AlmaLinux 9, systemd >=244, cgroup v2, selected boot entry and free space; retain package
  diff and Leapp logs.
- Verify console, public route, DNS, SSH, Tailscale, cloud-init and firewall backend/rules; prove
  CNI Pod-to-service traffic instead of assuming EL8 network settings migrated unchanged.
- Verify iSCSI sessions, Longhorn disks, volumes and replicas, plus controlled application I/O.
- Verify K3s server/etcd membership, CoreDNS, gateway and critical workloads. Do not upgrade K3s
  during this validation.
- Only then re-inspect containerd/K3s cgroup output against gVisor requirements. OS upgrade alone
  is not RuntimeClass acceptance.

## Rollback boundary and stop conditions

ELevate changes a broad userspace package set. Booting retained EL8 kernels does not restore EL8
systemd, libraries, network settings or K3s dependencies. The practical rollback is a tested cloud
disk/VM snapshot restore, or replacement/rebuild from known-good images and protected etcd plus
application backups. Restoring one control-plane disk after peers advance etcd can create a cluster
recovery incident; the datastore owner must choose the K3s restore procedure beforehand.

Stop for lost console access, unavailable off-node backup, failed quorum, Longhorn fault/degraded
or rebuilding state, insufficient replica capacity, an unresolved Leapp inhibitor, unreviewed
package removal, failed network/iSCSI recovery, or uncertainty about the authoritative datastore
restore path.

## Sources

- [AlmaLinux ELevate quickstart](https://wiki.almalinux.org/elevate/ELevate-quickstart-guide.html)
  and [migration paths](https://wiki.almalinux.org/elevate/)
- [gVisor systemd cgroup requirements](https://gvisor.dev/docs/user_guide/systemd/)
- [K3s etcd snapshots and restore](https://docs.k3s.io/cli/etcd-snapshot) and
  [manual upgrade behavior](https://docs.k3s.io/upgrades/manual)
- [Longhorn node eviction](https://longhorn.io/docs/1.12.1/nodes-and-volumes/nodes/disks-or-nodes-eviction/)
  and [production backups](https://longhorn.io/docs/1.12.1/best-practices/)
