# Scratch/rootfs runtime CI prototype

**Not a production runtime.** This isolated experiment leaves the controller,
RuntimeClasses, existing capacity eviction settings, node configuration, and all
user volumes unchanged. Build and acceptance run exclusively in
`.github/workflows/runtime-wrapper.yml` on a disposable privileged GitHub VM.
Do not run its fixture against a workstation or cluster node.

Native CI uses the pinned k3s `v1.36.2+k3s1` payload (SHA-256 checked) supplies the actual deployed
containerd `2.3.2-k3s2`, its `io.containerd.runc.v2` shim, and runc `1.4.2`. CRI
creates the sandbox, init, workspace and sidecar; this is not a direct-runc or
Docker substitute. Docker only constructs the fixture image, which is imported
into the separate containerd. Host kernel and XFS tools/features are recorded;
the GitHub kernel is **not** claimed identical to production.

## Narrow supported contract

A platform-owned `/etc/mwc-runtime-wrapper.json` selects a dedicated native
handler. Platform admission/RuntimeClass authorization remains a deployment
requirement; namespace annotations are not an independent authorization system.
The wrapper accepts only the current native, private overlay layout and current TopoLVM
thin storage. The original OCI `root.readonly` is preserved: developer roots stay
writable, while sidecar roots can be readonly with writable volume binds.
Rootless/bind/idmapped root snapshots,
checkpoint/restore/run, same-container-ID task reconstruction, unsupported mount
options, and other storage backends are rejected, not silently passed to an unbounded
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

The same scratch filesystem also holds the three current managed file binds:
`/etc/hosts`, `/etc/hostname`, and `/etc/resolv.conf`. The platform config supplies
the trusted CRI root and workspace namespace. Sources must match that sandbox's
exact CRI files or kubelet's exact Pod `etc-hosts` override; arbitrary host-file
overrides are rejected. Only each selected OCI `source` is patched in the complete
JSON document. Mount options and every other OCI field, including unknown fields
and sandbox security settings, survive unchanged. The small files retain their
initial contents, ownership, mode and xattrs; image layers are not copied.

Copies are shared by containers using the same source in one sandbox, preserving
cross-container writes. A replacement sandbox seeds fresh copies, including
kubelet hosts after a Pod IP change. These files remain private to the runtime
until the scratch volume is reclaimed: deleting one container must not unlink
another container's bound file. The current product's Pod hostname, hostAliases
and DNS settings are immutable; this does not implement a host-file update watcher
or arbitrary dynamic DNS/hosts reconfiguration. Individual-container deletion is
proven, while shared-file orphan GC remains outside this prototype.

At pause create, a trusted empty Pod log directory is replaced with a symlink to
`scratch/runtime-private/logs/<PodUID>`. The source must exactly match the
configured log root plus CRI namespace/name/UID. An existing correct link is
verified and reused; a nonempty directory or wrong link fails create. This runs
under the existing Pod lock before ordinary container loggers can open files.
No log bind mount is added. The rmdir/symlink interval is a short, recoverable
gap, not an atomic replacement. Single-container or sandbox deletion and failed
pause creation preserve the shared link and logs. Existing kubelet Pod log GC
owns unlinking the top-level link; scratch lifecycle owns reclaiming its target.

Logs are temporary storage. Active-Pod restarts and previous logs must work;
after the whole Pod ends and its scratch is unmounted, logs can become
unavailable. Persistent scratch bindings can retain old Pod-UID log directories
until their existing lifecycle owner clears them. No archive, log daemon, extra
mount hold, or historical backup is introduced. Log usage still contributes to
kubelet ephemeral statistics, whose reported capacity remains the node capacity.

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
same-sandbox managed-file sharing, original-host hashes, initial file metadata,
complete-OCI comparison against CRI's original runtimeSpec, DNS, fresh hosts in a
replacement sandbox, managed-file append ENOSPC on the full shared scratch,
copy-up/multiple-UID/sudo checks, normal delete, failed-executable cleanup, wrong-VG/metadata-symlink/ambiguous-
publish/namespace-mismatch/whole-scratch-root rejection and checkpoint rejection.
Dedicated CI handlers use external strace syscall injection (mount EIO,
unmount EBUSY, symlinkat EIO) to test shim cleanup after the switch fails; no wrapper test flag
or production fallback exists. These normal failed-create paths also exercise the
real shim's cleanup and runtime delete, which can safely remove the unused upper
after unmount. Arbitrary hard-kill/orphan recovery remains unimplemented; it is not
inferred from successful normal shim failure cleanup.
Unit input tests supplement this runtime path; they do not substitute for it.

The separate logs gate passed at `dc2dabf` in native CI run `40982775601`
(and its push check). It proves actual CRI stdout/stderr file descriptors on
scratch, overlapping sandboxes, rejected nonempty/wrong links, recoverable
symlink-creation failure, real kubelet API logs/TTY/follow, rotation/reopen/gzip,
active-Pod restart/previous logs, increased summary usage, shared log ENOSPC with
strict SSH/Home alive, and normal Pod GC unlinking only the source link. The
fixture owner normally unmounts its synthetic CSI publish after CRI tasks stop;
it does not install a real CSI driver or prove production CSI lifecycle.

The readonly sidecar gate uses the fixture image with real CRI readonly-root and
no-new-privileges settings, then a real kubelet Pod with the same constraints.
It checks root-file mutation/create EROFS, writable scratch/Home binds, absent
SYS_ADMIN, preserved complete OCI configuration, a writable main root, and normal
delete. This is not acceptance of the actual BuildKit/ttyd image composition.

Only CI results prove a gate passed. Source code and workflow intent do not.

## Independent runsc experiment

The `mwc_runsc` build selects a separate fixed config and strict runsc shim
parser; the default build remains runc. Shared trusted storage functions do not
select a runtime by filename or fall back to native. The real runsc
`release-20260831.0` full archive is SHA-256 pinned, including its adjacent
`gvisor-bin` sidecars and real `io.containerd.runsc.v1` shim. The isolated handler
uses grouping, `overlay2=none`, `file-access=shared` and shared mounts. Systrap is
explicit, OCI seccomp stays enabled, and OCI root readonly/capability fields
remain unchanged. No ignore-cgroups, allow-suid or privileged Pod workaround is
introduced. Four gVisor override annotation families and conflicting storage
flags are rejected; restore/checkpoint/run are unsupported.

A failed runsc create retains its record and overlay until that ID's real delete
succeeds. Runc's English absence error and a runsc list that skips unreadable
state are not used as cleanup authorization. Child create overlays remain
mounted through start, when the real child Gofer is created. Child delete only
removes that child's upper/work.

A separate CI matrix entry drives actual grouped pause/init/readonly sidecar and
writable main with independent Home. Its gates observe pre-start overlays,
Sentry/Gofer PIDs and seccomp, no-overlay Gofer configs and no filestore FDs,
ordinary/small writes, pwrite, fsync outcomes, mmap process failure with Sentry
alive, shared ENOSPC, Home/strict-key SSH, log capacity, child-only cleanup and
normal final unmount. SSH is between processes in the actual gVisor network
stack; external CNI ingress is not covered. Runsc results require their own
successful CI and independent receipt; native results prove none of them.

## Required remaining work before deployment

- Other writable host binds, including image-defined volumes, remain
  inventoried **outside this prototype's capacity coverage**. Actual current CRI has
  `ignoreImageDefinedVolumes=false`: actual current templates have no uncovered
  image VOLUME; Rust Dev's `/home/user` is overridden by Home. BuildKit rootless
  declares `/home/user/.local/share/buildkit`; the separate product PR88 adds an
  explicit existing-cache mount. Its full product runtime composition remains
  to be verified; no generic runtime volume-copy feature is introduced.
- Full wrapper/shim/containerd crashes, hard-kill transaction recovery, reboot,
  orphan ownership/GC, overlapping sandboxes and real CSI unpublish need a real
  lifecycle owner and acceptance. Visible mountinfo alone does not prove absence
  of every open/clone reference after crashes. The present fail-closed records do
  not implement recovery. A stale ID is explicitly rejected.
- The overlay snapshotter still tracks its old empty upper. Snapshot usage,
  commit/export, real kubelet statistics/eviction and thin-pool exhaustion are not
  repaired or proven. No existing ephemeral-storage threshold is removed.
- gVisor/runsc needs its independent matrix acceptance, preserving its sandbox;
  source implementation and native results do not prove those gates. Actual
  product images and complete real kubelet/CSI lifecycle remain separate work.
- BuildKit, complete product init/sidecar composition, apt package mutation,
  malicious identity/path replacement races and all fault boundaries need
  expanded real integration coverage. The initial fixture is a mechanism gate.

Passing this job supports a bounded native mechanism experiment, **not completion
of the product's two-storage-budget requirement or permission to deploy**.
