# Native scratch/rootfs CI prototype

**Not a production runtime.** This isolated experiment leaves the controller,
RuntimeClasses, existing capacity eviction settings, node configuration, and all
user volumes unchanged. Build and acceptance run exclusively in
`.github/workflows/runtime-wrapper.yml` on a disposable privileged GitHub VM.
Do not run its fixture against a workstation or cluster node.

The pinned k3s `v1.36.2+k3s1` payload (SHA-256 checked) supplies the actual deployed
containerd `2.3.2-k3s2`, its `io.containerd.runc.v2` shim, and runc `1.4.2`. CRI
creates the sandbox, init, workspace and sidecar; this is not a direct-runc or
Docker substitute. Docker only constructs the fixture image, which is imported
into the separate containerd. Host kernel and XFS tools/features are recorded;
the GitHub kernel is **not** claimed identical to production.

## Narrow supported contract

A platform-owned `/etc/mwc-runtime-wrapper.json` selects a dedicated native
handler. Platform admission/RuntimeClass authorization remains a deployment
requirement; namespace annotations are not an independent authorization system.
The wrapper accepts only the current native, writable, private overlay layout
and current TopoLVM thin storage. Readonly/rootless/bind/idmapped root snapshots,
checkpoint/restore/run, same-container-ID task reconstruction, unsupported mount
options, and legacy storage are rejected, not silently passed to an unbounded
rootfs. Ordinary runc features/state/start/exec/kill commands retain their real
protocol, descriptors, stdin/stdout/stderr and exit status.

The trusted node config contains the full scratch VG UUID and thin-pool DM UUID.
The resolver considers only the CRI Pod UID's actual kubelet CSI publish roots,
requires exactly one XFS scratch candidate, and links mount ID/device/inode,
root-owned CSI metadata, CSI volume handle, actual DM LV identity, thin target
table and expected pool. It rejects symlinks and writable ancestors in trusted
paths and revalidates immediately before use. No user annotation supplies host
paths. The user sees only the `data/` subtree; `runtime-private/<container-id>`
stays outside every scratch OCI mount, including kubelet subPath mounts.

Before real runc create, the same bundle/rootfs target is unmounted and remounted
with the original ordered lowerdirs and supported flags/options; only upper/work
move to scratch. Original empty-upper metadata and xattrs are copied. No nested
overlay, image copy, capability grant, security-context change or Home relocation
is performed. A failed replacement never launches against the original upper.
The rootfs mount and trusted scratch identity are recorded before mutation.

Create/delete supervision forwards signals and inherited descriptors. A failed
create is cleaned only after real runc reports that no task exists. Normal delete
waits for real runc success, unmounts the same rootfs target normally, checks visible
overlay references, then removes that container's upper/work. The outer real shim
performs its normal unmount afterwards. No lazy/force unmount or active-upper
recursive removal is used. A runtime/unmount/identity failure preserves state
and reports failure. Records are retained for CI evidence, not a production GC.

## CI evidence

The job builds a small independent scratch XFS on real loop-backed LVM thin,
an independent Home VG/XFS, and trusted CSI metadata/publish roots. Evidence
artifacts include actual OCI configs/writable mount inventory, wrapper records,
mountinfo, pinned runtime versions, kernel/XFS details, capacity before/after,
process ENOSPC/EDQUOT results, strict host-key SSH before/after full scratch,
copy-up/multiple-UID/sudo checks, normal delete and failed-executable cleanup.
Unit input tests supplement this runtime path; they do not substitute for it.

Only CI results prove a gate passed. Source code and workflow intent do not.

## Required remaining work before deployment

- CRI logs and writable host binds (`/etc/hosts`, hostname, resolver, image-defined
  volumes) remain inventoried **outside this prototype's capacity coverage**.
  Neither their redirection nor log rotation/reopen/previous/GC is implemented.
- Full wrapper/shim/containerd crashes, hard-kill transaction recovery, reboot,
  orphan ownership/GC, overlapping sandboxes and real CSI unpublish need a real
  lifecycle owner and acceptance. Visible mountinfo alone does not prove absence
  of every open/clone reference after crashes. The present fail-closed records do
  not implement recovery. A stale ID is explicitly rejected.
- The overlay snapshotter still tracks its old empty upper. Snapshot usage,
  commit/export, real kubelet statistics/eviction and thin-pool exhaustion are not
  repaired or proven. No existing ephemeral-storage threshold is removed.
- gVisor/runsc is a separate required implementation/acceptance, preserving its
  sandbox. This native-only job says nothing about runsc or all product images.
- BuildKit, complete product init/sidecar composition, apt package mutation,
  namespace mismatch, malicious identity/path races and all fault boundaries need
  expanded real integration coverage. The initial fixture is a mechanism gate.

Passing this job supports a bounded native mechanism experiment, **not completion
of the product's two-storage-budget requirement or permission to deploy**.
