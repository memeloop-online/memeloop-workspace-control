# serv-146231 gVisor kernel preparation

This is a preparation record, not an authorization to change the node. Do not install packages,
add a repository, change GRUB, cordon/drain, relabel, restart K3s, or reboot from this document
without a separately approved maintenance change.

## Executed maintenance result (2026-09-08)

After the separately approved ELRepo `5.15.220` one-shot boot, `serv-146231` returned Ready with
the 5.15.220 kernel. K3s `/readyz` was healthy; overseas Higress recovered (`2/2`) and the Wiki
recovered (`3/3`) with its volume healthy. The known-good AlmaLinux 4.18 kernel remains the GRUB
default, preserving the next-boot rollback route.

This is **not** final gVisor acceptance. At the time of this record, Longhorn volume
`pvc-6a03d5cb-b64b-43e1-b236-5bf3de83a613` remains attached to `haixia` but degraded. The old
serv replica stopped during the node outage after a connection reset/refusal. Longhorn has created
a replacement replica on `versetensor-hv`, but it is queued: that node's concurrent-rebuild limit
is one and another rebuild occupies the only slot. Do not delete either stopped replica, change
replica count, or raise rebuild concurrency as part of this kernel/gVisor work. Wait for the
existing rebuild to complete and observe the replacement becoming running and the volume becoming
healthy; if it does not, hand off to the storage owner as a separate incident.

## Observed state (2026-09-08)

| Item | Observation | Consequence |
| --- | --- | --- |
| Node | Kubernetes `serv-146231`; host `serv.146231.com` | Target identity must be checked on both planes before a change. |
| OS/kernel | AlmaLinux 8.10, `4.18.0-553.139.1.el8_10.x86_64` | Below gVisor's Linux 5.6 minimum; it must not receive the gVisor-ready label yet. |
| K3s | Server `v1.36.2+k3s1`, containerd `v2.3.2-k3s2` | This is a control-plane restart risk, not a worker-only experiment. |
| Boot | BIOS, BLS/GRUB; `GRUB_DEFAULT=saved`, `GRUB_TIMEOUT=0` | `grub2-reboot` is installed and one-shot selection is technically possible, but there is no interactive GRUB recovery window. Confirm provider serial/out-of-band reset first. |
| Capacity | `/boot` XFS: 610 MiB free; `/`: 11 GiB free | A transaction dry-run must prove the selected package payload and initramfs fit before install. |
| Drivers | `virtio_net`, `virtio_blk`, `virtio_console`, `xfs`, `iscsi_tcp` loaded; no DKMS/akmods/third-party kmod package was found | The new kernel still requires a post-boot check of all listed drivers and iSCSI. No external module reduces, but does not eliminate, risk. |
| Recovery | Root SSH and Tailscale/cloud-init services are enabled | This is not a substitute for a tested out-of-band console. A bad one-shot kernel can leave SSH unavailable until the provider resets the guest. |
| Storage | `iscsid` active with one session; Longhorn has two running replicas on this node; node-drain policy is `block-if-contains-last-replica` | Do not reboot until replica health, attachment state, backups, and safe evacuation are checked at the maintenance instant. |

The node currently serves overseas Higress controller/gateway, CoreDNS, a wiki Pod and Longhorn
components in addition to the K3s control-plane role. A kernel reboot interrupts all local Pods,
the node's API/etcd member, iSCSI and image pulls. This is materially higher risk than a normal
workspace-node canary.

## Kernel source decision

AlmaLinux 8's official BaseOS offers only its 4.18 stream (the latest observed candidate was
`4.18.0-553.159.1.el8_10`), so an AlmaLinux 8 update cannot satisfy gVisor's `uname` >= 5.6
requirement. An official AlmaLinux ELevate 8-to-9 upgrade is a separate OS-major migration; it
would provide AlmaLinux 9's 5.14 stream but is outside this node-level rollout and has no simple
GRUB one-shot rollback once packages are migrated.

The bounded alternative is ELRepo's signed EL8 `kernel-lt`, currently observed as
`5.15.220-1.el8.elrepo` (with matching core, modules and modules-extra RPMs). It installs beside,
not in place of, the AlmaLinux kernel and so can be tested with a one-shot BLS selection. ELRepo
explicitly calls `kernel-lt` a last resort, provides it as-is without vendor support/warranty, and
warns of security, performance and data-corruption risk. This is therefore a deliberate
third-party exception, not equivalent to AlmaLinux support. It is compatible with gVisor's stated
Linux 5.6+ prerequisite but does not prove workload compatibility.

Before use, independently compare the downloaded v2 ELRepo signing-key fingerprint with
`B8A7 5587 4DA2 40C9 DAC4 E715 5160 0989 EAA3 1D4A`. Do not bypass RPM GPG checking, use an
un-pinned `kernel-ml`, or make `elrepo-kernel` generally enabled for unrelated package upgrades.
Secure Boot is not currently the blocker (the host boots BIOS), but ELRepo says these kernels are
not Secure-Boot signed; re-evaluate if the boot mode changes.

## Staged-artifact and transaction evidence

The following files were downloaded on the target into the mode-0700 temporary directory
`/var/tmp/serv-146231-elrepo-kernel-lt.50FQsh`; they are **not installed** and no persistent DNF
repo was created. The directory is intentionally retained for the approval record and must be
re-hashed and signature-checked again immediately before any install.

| File | SHA-256 |
| --- | --- |
| `kernel-lt-5.15.220-1.el8.elrepo.x86_64.rpm` | `cacf5b97da283fb344bb5044ef9c04f6c38818b6de85ff5c0c731f95f041bb61` |
| `kernel-lt-core-5.15.220-1.el8.elrepo.x86_64.rpm` | `5f981c6aa6f732ad0d328ebe394a730a921623b84d7b72121222c5a357d9d21f` |
| `kernel-lt-modules-5.15.220-1.el8.elrepo.x86_64.rpm` | `5e1e2513544378941c846c03e91904a9829908c822724baf7bda4b3e85e932b9` |
| `kernel-lt-modules-extra-5.15.220-1.el8.elrepo.x86_64.rpm` | `31089fb7c8d8916ae0b40c5ba568952d44ac59980cc1be0c209a37cb1691a7a8` |
| `RPM-GPG-KEY-v2-elrepo.org` | `60789f228816a932251de2a3935f91829245366a465af183a9aa719b7693b494` |

The public key fingerprint matched the ELRepo-published v2 fingerprint and `rpmkeys`, using an
RPM database under that temporary directory, reported an OK RSA/SHA256 header and payload digest
for every staged RPM. The temporary DNF configuration also lives below that directory, uses only
the official `https://elrepo.org/linux/kernel/el8/$basearch/` metadata endpoint, and never writes
`/etc/yum.repos.d`.

Its real `--assumeno` transaction resolves exactly three packages: `kernel-lt`,
`kernel-lt-core`, and `kernel-lt-modules`, with a 92 MiB download and 130 MiB installed size; it
reported no removal or replacement of either installed AlmaLinux 4.18 kernel. The staged
`kernel-lt-modules-extra` is **not** a dependency of that transaction. RPM payload inspection
locates each currently required module (`iscsi_tcp`, `virtio_net`, `virtio_blk`,
`virtio_console`, and `xfs`) in `kernel-lt-core`, so modules-extra is not necessary for the
currently observed boot/iSCSI/Longhorn path. Do not install it speculatively.

## Approval gates

All gates must be fresh, recorded, and approved together:

1. Confirm etcd quorum and an etcd snapshot; confirm the other control-plane members are Ready.
2. Confirm a working provider serial console/out-of-band reset and a second independently tested
   SSH/Tailnet recovery route. `GRUB_TIMEOUT=0` means no human menu is available on a failed boot.
3. Cordon/evacuate only after verifying Longhorn can do so without a last healthy replica; wait
   for all affected volumes to be healthy, attached as expected, and backed up. The current
   Longhorn drain policy is a block, not permission to override it.
4. Record `df -h /boot /`, `grubby --info=ALL`, `grubby --default-kernel`, `grub2-editenv list`,
   `lsmod`, iSCSI sessions, and the K3s/Longhorn readiness result immediately before change.
5. Fetch and inspect the exact signed RPM transaction using `dnf --assumeno`; do not rely on this
   document's observed version after repository metadata has changed.

## Exact proposed procedure (after approval only)

These commands are intentionally explicit and sequential. Substitute nothing for the captured
values, and stop on any unexpected output.

```bash
# 0. Capture the old default before any RPM action.
old_kernel=$(grubby --default-kernel)
grubby --info=ALL
grub2-editenv list
df -h /boot /

# 1. Re-hash and signature-check the previously approved staged files. This does not import a
#    key into the system RPM database or add an /etc repository.
stage=/var/tmp/serv-146231-elrepo-kernel-lt.50FQsh
(cd "$stage" && sha256sum -c <<'EOF'
cacf5b97da283fb344bb5044ef9c04f6c38818b6de85ff5c0c731f95f041bb61  kernel-lt-5.15.220-1.el8.elrepo.x86_64.rpm
5f981c6aa6f732ad0d328ebe394a730a921623b84d7b72121222c5a357d9d21f  kernel-lt-core-5.15.220-1.el8.elrepo.x86_64.rpm
5e1e2513544378941c846c03e91904a9829908c822724baf7bda4b3e85e932b9  kernel-lt-modules-5.15.220-1.el8.elrepo.x86_64.rpm
60789f228816a932251de2a3935f91829245366a465af183a9aa719b7693b494  RPM-GPG-KEY-v2-elrepo.org
EOF
)
rpmkeys --dbpath "$stage/rpmdb" --checksig --verbose "$stage"/*.rpm

# 2. Repeat the no-write, temporary-repo transaction check. It must resolve exactly three RPMs
#    and no removals. --assumeno returns 1 after displaying the resolved transaction.
dnf --config="$stage/dnf.conf" --disablerepo='*' --enablerepo=elrepo-kernel --assumeno \
  install kernel-lt-5.15.220-1.el8.elrepo

# 3. Install only the three locally verified, exact RPMs. No persistent ELRepo repo is needed.
dnf --disablerepo='*' install \
  "$stage/kernel-lt-5.15.220-1.el8.elrepo.x86_64.rpm" \
  "$stage/kernel-lt-core-5.15.220-1.el8.elrepo.x86_64.rpm" \
  "$stage/kernel-lt-modules-5.15.220-1.el8.elrepo.x86_64.rpm"

# 4. The approved transaction installs exactly these NEVRAs; modules-extra is not required.
#    Record `rpm -q` output after the transaction before proceeding.
rpm -q kernel-lt-5.15.220-1.el8.elrepo.x86_64 \
  kernel-lt-core-5.15.220-1.el8.elrepo.x86_64 \
  kernel-lt-modules-5.15.220-1.el8.elrepo.x86_64

# 5. Identify the new BLS entry by its kernel path; never select a numeric index.
new_kernel=/boot/vmlinuz-5.15.220-1.el8.elrepo.x86_64
grubby --info="$new_kernel"
new_id=$(grubby --info="$new_kernel" | awk -F= '/^id=/{gsub(/"/, "", $2); print $2}')
test -n "$new_id"

# 6. Preserve the known-good 4.18 default, then select 5.15 for the next boot only.
grubby --set-default "$old_kernel"
grub2-reboot "$new_id"
grub2-editenv list

# 7. Reboot only after the maintenance lead confirms quorum, replica safety and console access.
systemctl reboot
```

The procedure uses a version-pinned, locally verified kernel RPM set and does not retain an ELRepo
repository configuration. It does not change the normal K3s runtime or add a Kubernetes
RuntimeClass.

## First-boot acceptance and rollback

On the one-shot 5.15 boot, prove the kernel, root/boot mounts, default route, SSH/Tailscale,
`iscsid`, `k3s`, containerd, etcd/member health, Longhorn node/replica health, CoreDNS and
overseas gateway before enabling gVisor. Check the required drivers with
`lsmod | egrep 'virtio_(net|blk|console)|xfs|iscsi_tcp'` and verify the active iSCSI session.
Keep the old 4.18 kernel as default throughout the canary.

If the system is reachable but any check fails, boot the old kernel for the next boot and reboot
only with the maintenance lead's approval:

```bash
grubby --set-default "$old_kernel"
systemctl reboot
```

If the new kernel does not boot or networking fails, use the pre-verified provider console/reset.
Because the normal default remains 4.18, the subsequent boot should select it after `next_entry`
is consumed; do not assume this repairs a guest that is hung before reboot. Once back on 4.18 and
all cluster/storage checks are healthy, remove the tested ELRepo packages only after confirming
the running kernel is not `kernel-lt`:

```bash
uname -r  # must show the retained AlmaLinux 4.18 kernel, never kernel-lt
dnf remove \
  kernel-lt-5.15.220-1.el8.elrepo.x86_64 \
  kernel-lt-core-5.15.220-1.el8.elrepo.x86_64 \
  kernel-lt-modules-5.15.220-1.el8.elrepo.x86_64
grubby --info=ALL
```

If a later approved transaction explicitly installs `kernel-lt-modules-extra`, record its exact
NEVRA and add only that exact package to the removal command. Never use a wildcard removal.

Do not remove the AlmaLinux kernel packages or rescue entries. Removal of the ELRepo release RPM
and GPG key is a separate policy decision after confirming no remaining ELRepo package depends on
them.

References: [gVisor installation requirements](https://gvisor.dev/docs/user_guide/install/),
[ELRepo kernel-lt policy](https://elrepo.org/wiki/doku.php?id=kernel-lt),
[ELRepo signing key](https://elrepo.org/wiki/doku.php?id=key),
[AlmaLinux ELevate quickstart](https://wiki.almalinux.org/elevate/ELevate-quickstart-guide.html),
and [RHEL 8 GRUB/kernel management](https://docs.redhat.com/en/documentation/red_hat_enterprise_linux/8/html/managing_monitoring_and_updating_the_kernel/managing_monitoring_and_updating_the_kernel).
