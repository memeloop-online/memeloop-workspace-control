#!/bin/bash
# Disposable privileged GitHub VM only. Never invoke against a cluster/node.
set -Eeuo pipefail
umask 022
: "${GITHUB_ACTIONS:?This fixture only runs on a disposable GitHub Actions VM}"
: "${RUNNER_TEMP:?}"
mode="${1:-native}"
case "$mode" in native|runsc) ;; *) echo "unsupported fixture runtime: $mode"; exit 1;; esac
base="/var/lib/mwc-runtime-prototype"
artifacts="$GITHUB_WORKSPACE/runtime-wrapper/artifacts"
mkdir -p "$base" "$artifacts"
install -m 0755 "$RUNNER_TEMP/mwc-runtime-prototype/mwc-runtime-wrapper" "$base/mwc-runtime-wrapper"
if [ "$mode" = runsc ]; then install -m 0755 "$RUNNER_TEMP/mwc-runtime-prototype/mwc-runsc-wrapper" "$base/mwc-runsc-wrapper"; fi
exec > >(tee "$artifacts/fixture.log") 2>&1
export PATH="$base/k3s/data/current/bin:$PATH"
uid=11111111-2222-4333-8444-555555555555
scratch_handle=aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee
home_handle=bbbbbbbb-cccc-4ddd-8eee-ffffffffffff
scratch_pv=pvc-22222222-3333-4444-8555-666666666666
home_pv=pvc-33333333-4444-4555-8666-777777777777
vg=mwc-ci-scratch
home_vg=mwc-ci-home
podroot="$base/kubelet/pods/$uid/volumes/kubernetes.io~csi"
scratch="$podroot/$scratch_pv/mount"
home="$podroot/$home_pv/mount"
containerd_pid=''
k3s_pid=''
kubelet_publish=''
sandbox=''
containers=()
loops=()
cleanup() {
 set +e
 trap - ERR
 if [ -f "$base/k3s-control.log" ]; then cp "$base/k3s-control.log" "$artifacts/k3s-control.log"; fi
 if [ "$mode" = runsc ] && [ -d "$base/runsc-state" ]; then
  mkdir -p "$artifacts/failure-runsc-bundles"
  # Exact state names contain ':'. Archive regular files so artifact upload
  # preserves them without trying to copy namespace handles or control sockets.
  find "$base/runsc-state" -printf '%y %p\n' > "$artifacts/failure-runsc-state-files.txt"
  (cd "$base/runsc-state" && find . -type f \( -name '*.state' -o -name '*.lock' \) -print0 | /usr/bin/tar --null -T - -cf "$artifacts/failure-runsc-state.tar")
  for bundle in "$base/containerd-state/io.containerd.runtime.v2.task/k8s.io/"*; do
   [ -d "$bundle" ] || continue
   dest="$artifacts/failure-runsc-bundles/$(basename "$bundle")"; mkdir -p "$dest"
   for name in log.json config.json init.pid; do [ ! -f "$bundle/$name" ] || cp "$bundle/$name" "$dest/"; done
  done
  ps -eo pid,ppid,args > "$artifacts/failure-runsc-processes.txt"
  for id in "${containers[@]}"; do
   if cri inspect "$id" > "$artifacts/failure-runsc-$id-inspect.json" 2>/dev/null; then
    cri logs "$id" > "$artifacts/failure-runsc-$id-log.txt" 2>&1
   fi
  done
  for id in $(cri ps -a --label "io.kubernetes.pod.uid=$uid" -q); do cri stop "$id"; cri rm "$id"; done
  for id in $(cri pods --label "io.kubernetes.pod.uid=$uid" -q); do cri stopp "$id"; cri rmp "$id"; done
 fi
 if [ -n "$k3s_pid" ] && declare -F kube >/dev/null; then
  kube get pod -n memeloop-workspace-control mwc-log-ci -o json | jq '{metadata:{uid:.metadata.uid,deletionTimestamp:.metadata.deletionTimestamp},status}' > "$artifacts/kubelet-final-pod-status.json"
  kube get events -n memeloop-workspace-control -o json | jq '[.items[]|{reason,message,type}]' > "$artifacts/kubelet-final-events.json"
 fi
 if [ -n "$k3s_pid" ]; then kill "$k3s_pid"; wait "$k3s_pid"; k3s_pid=''; fi
 if [ -n "${kube_uid:-}" ]; then
  for id in $(cri ps -a --label "io.kubernetes.pod.uid=$kube_uid" -q); do cri stop "$id"; cri rm "$id"; done
  for id in $(cri pods --label "io.kubernetes.pod.uid=$kube_uid" -q); do cri stopp "$id"; cri rmp "$id"; done
 fi
 for id in "${containers[@]}"; do cri stop "$id"; cri rm "$id"; done
 if [ -n "$sandbox" ]; then cri stopp "$sandbox"; cri rmp "$sandbox"; fi
 cat /proc/self/mountinfo > "$artifacts/final-mountinfo.txt"
 if [ -d "$base/state" ]; then cp -a "$base/state" "$artifacts/failure-or-final-state"; fi
 if [ -f "$base/state/errors.log" ]; then cat "$base/state/errors.log"; fi
 if [ -n "$kubelet_publish" ]; then umount "$kubelet_publish"; fi
 if [ -n "$containerd_pid" ]; then kill "$containerd_pid"; wait "$containerd_pid"; fi
 # Normal unmounts only. EBUSY is evidence/failure, never hidden by lazy/force.
 if [ "${scratch_detached:-0}" = 0 ]; then umount "$scratch"; fi
 if [ "${home_detached:-0}" = 0 ]; then umount "$home"; fi
 vgremove -fy "$vg"; vgremove -fy "$home_vg"
 for loop in "${loops[@]}"; do losetup -d "$loop"; done
}
trap cleanup EXIT
trap 'status=$?; printf "status=%s source=%s line=%s command=%s\n" "$status" "${BASH_SOURCE[0]}" "$LINENO" "$BASH_COMMAND" >> "$artifacts/failed-command.txt"' ERR
cri() { crictl --runtime-endpoint "unix://$base/containerd.sock" --image-endpoint "unix://$base/containerd.sock" "$@"; }
ctr_ci() { ctr --address "$base/containerd.sock" --namespace k8s.io "$@"; }

# Immutable deployed payload. Its embedded containerd/shim/runc are used together.
curl --fail --location --retry 3 'https://github.com/k3s-io/k3s/releases/download/v1.36.2%2Bk3s1/k3s' -o "$base/k3s-bin"
echo "65a55ec56c24eab44383086166ec620a491952b7e23941a49ddca6e8a4c4b4de  $base/k3s-bin" | sha256sum --check
chmod 0755 "$base/k3s-bin"
K3S_DATA_DIR="$base/k3s" "$base/k3s-bin" ctr version --help >/dev/null
containerd --version | tee "$artifacts/containerd-version.txt"
runc --version | tee "$artifacts/runc-version.txt"
grep -F 'v2.3.2-k3s2' "$artifacts/containerd-version.txt"
grep -E '^runc version 1\.4\.2$' "$artifacts/runc-version.txt"
uname -a > "$artifacts/kernel.txt"
mkfs.xfs -V > "$artifacts/xfs-tools.txt" 2>&1

# Separate real thin VGs make Home-vs-scratch discrimination meaningful.
for kind in scratch home; do
 truncate -s 3G "$base/$kind.img"
 loop="$(losetup --find --show "$base/$kind.img")"; loops+=("$loop")
 pvcreate -ff -y "$loop"
 if [ "$kind" = scratch ]; then this_vg="$vg"; handle="$scratch_handle"; target="$scratch"; else this_vg="$home_vg"; handle="$home_handle"; target="$home"; fi
 vgcreate "$this_vg" "$loop"
 lvcreate -L 2G --poolmetadatasize 16M -T "$this_vg/pool"
 lvcreate -V 512M -T "$this_vg/pool" -n "$handle"
 mkfs.xfs -f "/dev/$this_vg/$handle"
 mkdir -p "$target"
 mount -t xfs "/dev/$this_vg/$handle" "$target"
 chmod 0755 "$target"
 xfs_info "$target" > "$artifacts/$kind-xfs-info.txt"
 handle_json="$(printf '%s' "$handle")"
 pv_json="$(basename "$(dirname "$target")")"
 jq -n --arg handle "$handle_json" --arg pv "$pv_json" '{driverName:"topolvm.io",volumeHandle:$handle,specVolID:$pv,volumeLifecycleMode:"Persistent"}' > "$(dirname "$target")/vol_data.json"
done
mkdir -p "$scratch/data/tmp" "$scratch/data/build-cache" "$home/.ssh"
chmod 1777 "$scratch/data/tmp"; chmod 0777 "$scratch/data" "$scratch/data/build-cache"
chown -R 1000:1000 "$home"
# Only scratch root is node-owned/protected; Home belongs to the user.
ssh-keygen -q -t ed25519 -N '' -f "$base/client"
ssh-keygen -q -t ed25519 -N '' -f "$home/host_ed25519"
cp "$base/client.pub" "$home/authorized_keys"
chmod 0644 "$home/authorized_keys"
chown 1000:1000 "$home/authorized_keys"
printf '[127.0.0.1]:2222 %s\n' "$(cut -d ' ' -f 1,2 "$home/host_ed25519.pub")" > "$base/known_hosts"
# The fixture account must be unlocked for public-key-only SSH.
# It receives no host capabilities; normal OCI defaults still apply.

docker build -t mwc-runtime-fixture:ci "$GITHUB_WORKSPACE/runtime-wrapper/ci"
docker save mwc-runtime-fixture:ci -o "$base/image.tar"
mkdir -p "$base/containerd-root/io.containerd.snapshotter.v1.overlayfs/snapshots" "$base/containerd-state/io.containerd.runtime.v2.task/k8s.io" "$base/state" "$base/kubelet" "$base/pod-logs"
chmod 0700 "$base/state"
vg_uuid="$(vgs --noheadings -o vg_uuid "$vg" | tr -d '[:space:]-')"
pool_uuid="$(dmsetup info -c --noheadings -o uuid "${vg//-/--}-pool-tpool" | xargs)"
real_runc="$(readlink -f "$(command -v runc)")"
jq -n --arg runtime "$real_runc" --arg logroot "$base/pod-logs" --arg crista "$base/containerd-state/io.containerd.grpc.v1.cri" --arg criroot "$base/containerd-root/io.containerd.grpc.v1.cri" --arg kubelet "$base/kubelet" --arg bundle "$base/containerd-state/io.containerd.runtime.v2.task/k8s.io" --arg snapshot "$base/containerd-root/io.containerd.snapshotter.v1.overlayfs/snapshots" --arg state "$base/state" --arg namespace "/proc/$$/ns/mnt" --arg vg "$vg_uuid" --arg pool "$pool_uuid" --arg dmsetup "$(readlink -f "$(command -v dmsetup)")" '{real_runtime:$runtime,kubelet_root:$kubelet,cri_root:$criroot,cri_state_root:$crista,workspace_namespace:"memeloop-workspace-control",pod_log_root:$logroot,bundle_root:$bundle,snapshot_root:$snapshot,state_root:$state,mount_namespace:$namespace,scratch_vg_uuid:$vg,scratch_pool_dm_uuid:$pool,dmsetup:$dmsetup}' > /etc/mwc-runtime-wrapper.json
chmod 0600 /etc/mwc-runtime-wrapper.json
cat > "$base/containerd.toml" <<TOML
version = 3
root = "$base/containerd-root"
state = "$base/containerd-state"
[grpc]
  address = "$base/containerd.sock"
[plugins.'io.containerd.cri.v1.images']
  snapshotter = 'overlayfs'
  [plugins.'io.containerd.cri.v1.images'.pinned_images]
    sandbox = 'registry.k8s.io/pause:3.10'
[plugins.'io.containerd.cri.v1.runtime']
  [plugins.'io.containerd.cri.v1.runtime'.containerd]
    default_runtime_name = 'mwc-ci'
    [plugins.'io.containerd.cri.v1.runtime'.containerd.runtimes.'mwc-ci']
      runtime_type = 'io.containerd.runc.v2'
      [plugins.'io.containerd.cri.v1.runtime'.containerd.runtimes.'mwc-ci'.options]
        BinaryName = '$base/mwc-runtime-wrapper'
        SystemdCgroup = false
TOML
# Fault injection belongs to the disposable harness, never to wrapper options.
for fault in mount unmount loglink; do
 if [ "$fault" = mount ]; then injection='mount:error=EIO:when=1'; elif [ "$fault" = unmount ]; then injection='umount2:error=EBUSY:when=1'; else injection='symlinkat:error=EIO:when=1'; fi
 cat > "$base/fail-$fault-runtime" <<SCRIPT
#!/bin/sh
exec /usr/bin/strace -ff -qq -o '$artifacts/fault-$fault-syscalls.txt' -e trace=mount,umount2,rmdir,symlink,symlinkat -e inject=$injection '$base/mwc-runtime-wrapper' "\$@"
SCRIPT
 chmod 0755 "$base/fail-$fault-runtime"
 cat >> "$base/containerd.toml" <<TOML
    [plugins.'io.containerd.cri.v1.runtime'.containerd.runtimes.'mwc-ci-fail-$fault']
      runtime_type = 'io.containerd.runc.v2'
      [plugins.'io.containerd.cri.v1.runtime'.containerd.runtimes.'mwc-ci-fail-$fault'.options]
        BinaryName = '$base/fail-$fault-runtime'
        SystemdCgroup = false
TOML
done
if [ "$mode" = runsc ]; then
 source "$GITHUB_WORKSPACE/runtime-wrapper/ci/runsc-setup.sh"
fi
containerd --config "$base/containerd.toml" > "$artifacts/containerd.log" 2>&1 &
containerd_pid=$!
for n in $(seq 1 60); do if cri info > "$artifacts/cri-info.json"; then break; fi; sleep 1; done
cri info > "$artifacts/cri-info.json"
ctr_ci images import "$base/image.tar"
cri pull registry.k8s.io/pause:3.10
if [ "$mode" = runsc ]; then
 source "$GITHUB_WORKSPACE/runtime-wrapper/ci/runsc-fixture.sh"
 df -B1 "$scratch" "$home" > "$artifacts/after-container-delete-df.txt"
 cp -a "$base/state" "$artifacts/final-wrapper-state"
 exit 0
fi
pod_log="$base/pod-logs/memeloop-workspace-control_mwc-budget-ci_$uid"
jq -n --arg uid "$uid" --arg log "$pod_log" '{metadata:{name:"mwc-budget-ci",namespace:"memeloop-workspace-control",uid:$uid,attempt:0},log_directory:$log,linux:{security_context:{namespace_options:{network:2}}}}' > "$base/pod.json"
mkdir -p "$pod_log"
sandbox="$(cri runp --runtime mwc-ci "$base/pod.json")"
containers+=("$sandbox")

kubelet_hosts="$base/kubelet/pods/$uid/etc-hosts"
printf '127.0.0.1 localhost mwc-budget.local\n::1 localhost\n' > "$kubelet_hosts"
python3 - "$kubelet_hosts" <<'PYX'
import os,sys
os.setxattr(sys.argv[1],"user.mwc-fixture",b"preserve-xattr")
PYX
managed_host_root="$base/containerd-root/io.containerd.grpc.v1.cri/sandboxes/$sandbox"
sha256sum "$kubelet_hosts" "$managed_host_root/hostname" "$managed_host_root/resolv.conf" > "$artifacts/managed-host-before.sha256"

make_container() {
 local name="$1"; shift
 jq -n --arg name "$name" --arg home "$home" --arg data "$scratch/data" --arg hosts "$kubelet_hosts" --args '{metadata:{name:$name,attempt:0},image:{image:"docker.io/library/mwc-runtime-fixture:ci"},command:["/usr/local/bin/mwc-budget-workload"],args:$ARGS.positional,log_path:($name+"/0.log"),mounts:[{container_path:"/etc/hosts",host_path:$hosts},{container_path:"/home/user",host_path:$home},{container_path:"/scratch",host_path:$data},{container_path:"/tmp",host_path:($data+"/tmp")},{container_path:"/var/tmp",host_path:($data+"/tmp")}],linux:{security_context:{capabilities:{drop_capabilities:["SYS_ADMIN"]},no_new_privs:($name=="ttyd"),readonly_rootfs:($name=="ttyd")}}}' "$@" > "$base/$name.json"
 local id
 id="$(cri create "$sandbox" "$base/$name.json" "$base/pod.json")"
 containers+=("$id"); last_id="$id"
 cp "$base/containerd-state/io.containerd.runtime.v2.task/k8s.io/$id/config.json" "$artifacts/$name-oci.json" 2>/dev/null || true
 cri start "$id"
}
test -L "$pod_log"
test "$(readlink "$pod_log")" = "$scratch/runtime-private/logs/$uid"
# Overlapping sandboxes of the same Pod must share the exact log target.
jq '.metadata.attempt=1' "$base/pod.json" > "$base/overlap-pod.json"
overlap_id="$(cri runp --runtime mwc-ci "$base/overlap-pod.json")"
containers+=("$overlap_id")
test "$(readlink "$pod_log")" = "$scratch/runtime-private/logs/$uid"
cri stopp "$overlap_id"; cri rmp "$overlap_id"
test -L "$pod_log"
make_container init init
init_id="$last_id"
for n in $(seq 1 20); do if [ "$(cri inspect "$init_id" | jq -r .status.state)" = CONTAINER_EXITED ]; then break; fi; sleep 1; done
cri inspect "$init_id" > "$artifacts/init-status.json"
test "$(jq -r .status.exitCode "$artifacts/init-status.json")" = 0
# A completed init task is normally reaped by the real CRI event path.
make_container ttyd idle
ttyd_id="$last_id"
make_container workspace serve
main_id="$last_id"
# Capture the *actual* shim generated OCI config after start, not handcrafted spec.
for pair in "sandbox:$sandbox" "init:$init_id" "ttyd:$ttyd_id" "workspace:$main_id"; do
 name="${pair%%:*}"; id="${pair#*:}"
 cp "$base/state/$id.json" "$artifacts/$name-wrapper.json"
 if [ -f "$base/containerd-state/io.containerd.runtime.v2.task/k8s.io/$id/config.json" ]; then cp "$base/containerd-state/io.containerd.runtime.v2.task/k8s.io/$id/config.json" "$artifacts/$name-oci.json"; fi
done
cri inspect "$main_id" > "$artifacts/workspace-inspect.json"
cri inspect "$ttyd_id" > "$artifacts/ttyd-inspect.json"
python3 "$GITHUB_WORKSPACE/runtime-wrapper/ci/managed-files.py" "$artifacts"
cri exec "$ttyd_id" /usr/local/bin/mwc-budget-workload readonly-check > "$artifacts/readonly-sidecar-process.json"
cri exec "$main_id" cat /scratch/readonly-marker /home/user/readonly-marker > "$artifacts/readonly-sidecar-shared-markers.txt"
for pair in "init:$init_id" "workspace:$main_id" "sidecar:$ttyd_id"; do
 name="${pair%%:*}"; id="${pair#*:}"
 cri logs "$id" > "$artifacts/log-$name.txt" 2>&1
 grep -F "$name-stdout-start" "$artifacts/log-$name.txt"
 grep -F "$name-stderr-start" "$artifacts/log-$name.txt"
done
python3 - "$pod_log" "$scratch" "$containerd_pid" "$artifacts" <<'PYX'
import json,os,pathlib,sys
log,scratch,pid,out=map(pathlib.Path,sys.argv[1:])
dev=scratch.stat().st_dev
files=list(log.glob('*/*.log'))
assert len(files)>=3,files
assert all(f.stat().st_dev==dev for f in files)
fds=[]
for fd in pathlib.Path('/proc',str(pid),'fd').iterdir():
 try:
  if fd.stat().st_dev==dev and '/runtime-private/logs/' in os.readlink(fd):fds.append(os.readlink(fd))
 except FileNotFoundError:pass
assert len(fds)>=2,fds
(out/'real-cri-log-files.json').write_text(json.dumps({'files':[str(f.resolve()) for f in files],'containerd_open_log_fds':fds,'device':dev},indent=2))
PYX
# Both containers see the same managed inodes; node-managed originals stay intact.
cri exec "$main_id" bash -c 'set -e; for name in hosts hostname resolv.conf; do printf "\n# shared-%s\n" "$name" >> "/etc/$name"; done; sync'
cri exec "$ttyd_id" bash -c 'set -e; for name in hosts hostname resolv.conf; do grep -F "# shared-$name" "/etc/$name"; done; getent hosts mwc-budget.local; getent ahostsv4 registry.k8s.io' > "$artifacts/managed-shared-dns.txt"
sha256sum --check "$artifacts/managed-host-before.sha256" > "$artifacts/managed-host-after-append.txt"
cat /proc/self/mountinfo > "$artifacts/active-mountinfo.txt"
ssh_ci() { ssh -i "$base/client" -p 2222 -o BatchMode=yes -o StrictHostKeyChecking=yes -o UserKnownHostsFile="$base/known_hosts" -o ConnectTimeout=5 user@127.0.0.1 "$@"; }
for n in $(seq 1 20); do if ssh_ci 'id'; then break; fi; sleep 1; done
ssh_ci 'set -e; test "$(sudo -n id -u)" = 0; test ! -e /scratch/runtime-private; test ! -e /runtime-private; touch /home/user/ssh-marker; echo strict-ssh-home-sudo-ok' | tee "$artifacts/ssh-before-full.txt"
# Copy-up from immutable image, multi UID, and sidecar allocation consume same FS.
lower_seed="$(jq -r '.lower | split(":")[0]' "$base/state/$main_id.json")/seed-copy-up"
sha256sum "$lower_seed" > "$artifacts/lower-seed-before.sha256"
cri exec "$main_id" bash -c 'set -e; printf changed >> /seed-copy-up; sudo -u user touch /home/user/multi-uid; dd if=/dev/zero of=/scratch/data-initial bs=1M count=32 conv=fsync'
cri exec "$ttyd_id" bash -c 'dd if=/dev/zero of=/scratch/sidecar-allocation bs=1M count=16 conv=fsync'
sha256sum "$lower_seed" > "$artifacts/lower-seed-after.sha256"
cmp "$artifacts/lower-seed-before.sha256" "$artifacts/lower-seed-after.sha256"
df -B1 "$scratch" "$home" > "$artifacts/before-full-df.txt"
cri exec "$main_id" /usr/local/bin/mwc-budget-workload fill | tee "$artifacts/full-result.txt"
python3 - "$artifacts/full-result.txt" <<'PYX'
import errno,json,sys
r=json.load(open(sys.argv[1]))
for path in ['/etc/hosts','/etc/hostname','/etc/resolv.conf']:
 assert r[path]['errno'] in (errno.ENOSPC,errno.EDQUOT),r
PYX
sha256sum --check "$artifacts/managed-host-before.sha256" > "$artifacts/managed-host-after-full.txt"
ssh_ci 'set -e; test -f /home/user/home-still-writable; sudo -n sh -c "echo ssh-alive > /home/user/after-full"; echo strict-ssh-after-full-home-writable' > "$artifacts/ssh-after-full.txt"
cri inspect "$main_id" > "$artifacts/main-after-full.json"
test "$(jq -r .status.state "$artifacts/main-after-full.json")" = CONTAINER_RUNNING
cri exec "$main_id" bash -c 'set -e; rm /rootfs-fill /scratch/data-fill /scratch/data-initial; echo recovered > /rootfs-after-free; sync'
df -B1 "$scratch" "$home" > "$artifacts/after-delete-df.txt"
# Negative gates use real CRI starts and ordinary root-owned fixture config.
# No test-only switch or fallback path is added to the production resolver.
cp /etc/mwc-runtime-wrapper.json "$base/trusted-config.json"
negative_start() {
 local name="$1" expected="$2" config_file="${3:-$base/workspace.json}"
 local id before
 before="$(wc -l < "$base/state/errors.log" 2>/dev/null || printf '0')"
 jq --arg name "$name" '.metadata.name=$name' "$config_file" > "$base/negative-$name.json"
 id="$(cri create "$sandbox" "$base/negative-$name.json" "$base/pod.json")"
 containers+=("$id")
 if cri start "$id" > "$artifacts/negative-$name-start.txt" 2>&1; then echo "negative $name unexpectedly started"; exit 1; fi
 tail -n "+$((before+1))" "$base/state/errors.log" > "$artifacts/negative-$name-errors.txt"
 grep -F "$expected" "$artifacts/negative-$name-errors.txt"
 test ! -e "$scratch/runtime-private/$id"
 ! grep -F "/$id/rootfs " /proc/self/mountinfo
 cri rm "$id"
 jq -cn --arg name "$name" --arg id "$id" '{gate:$name,container_id:$id,rejected:true,rootfs_mount_leak:false,private_upper_created:false}' >> "$artifacts/negative-results.jsonl"
}
# A different protected VG cannot be accepted merely because it is XFS.
jq '.scratch_vg_uuid="ZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZ" | .scratch_pool_dm_uuid="LVM-ZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZYYYYYYYYYYYYYYYYYYYYYYYYYYYYYYYY-tpool"' "$base/trusted-config.json" > /etc/mwc-runtime-wrapper.json
negative_start wrong-vg 'expected exactly one trusted scratch publish, got 0'
cp "$base/trusted-config.json" /etc/mwc-runtime-wrapper.json
# CSI metadata must be opened without following a symlink.
mv "$(dirname "$scratch")/vol_data.json" "$base/saved-vol-data.json"
ln -s "$base/saved-vol-data.json" "$(dirname "$scratch")/vol_data.json"
negative_start metadata-symlink 'vol_data.json'
rm "$(dirname "$scratch")/vol_data.json"
mv "$base/saved-vol-data.json" "$(dirname "$scratch")/vol_data.json"
# Two valid candidates are ambiguous even if they alias the same LV.
ambiguous_pv=pvc-44444444-5555-4666-8777-888888888888
ambiguous="$podroot/$ambiguous_pv/mount"
mkdir -p "$ambiguous"
mount --bind "$scratch" "$ambiguous"
jq --arg pv "$ambiguous_pv" '.specVolID=$pv' "$(dirname "$scratch")/vol_data.json" > "$(dirname "$ambiguous")/vol_data.json"
negative_start ambiguous-publish 'expected exactly one trusted scratch publish, got 2'
umount "$ambiguous"
rm -r "$(dirname "$ambiguous")"
# A genuine namespace mismatch must reject before touching the rootfs.
unshare --mount sleep 60 &
other_namespace_pid=$!
for n in $(seq 1 20); do
 if [ "$(readlink "/proc/$other_namespace_pid/ns/mnt")" != "$(readlink /proc/self/ns/mnt)" ]; then break; fi
 sleep .05
done
jq --arg namespace "/proc/$other_namespace_pid/ns/mnt" '.mount_namespace=$namespace' "$base/trusted-config.json" > /etc/mwc-runtime-wrapper.json
negative_start namespace-mismatch 'mount namespaces differ'
cp "$base/trusted-config.json" /etc/mwc-runtime-wrapper.json
kill "$other_namespace_pid"
wait "$other_namespace_pid" || true
# A container-root user must not receive the scratch volume root/private tree.
jq --arg root "$scratch" '(.mounts[] | select(.container_path=="/home/user")).host_path=$root' "$base/workspace.json" > "$base/exposes-root.json"
negative_start exposed-scratch-root 'scratch OCI mount exposes volume root/private area' "$base/exposes-root.json"
# No arbitrary user-supplied /etc source may enter the host-copy path.
jq --arg source "$base/trusted-config.json" '(.mounts[] | select(.container_path=="/etc/hosts")).host_path=$source' "$base/workspace.json" > "$base/untrusted-hosts.json"
negative_start untrusted-hosts 'untrusted managed file source for /etc/hosts' "$base/untrusted-hosts.json"
if "$base/mwc-runtime-wrapper" checkpoint "$main_id" > "$artifacts/checkpoint-stdout.txt" 2> "$artifacts/checkpoint-stderr.txt"; then echo 'checkpoint bypass unexpectedly accepted'; exit 1; else test "$?" = 125; fi
test ! -s "$artifacts/checkpoint-stdout.txt"
"$base/mwc-runtime-wrapper" features > "$artifacts/real-runc-features.json"
jq -e '.ociVersionMax | startswith("1.")' "$artifacts/real-runc-features.json"

# Invalid real-runc create must not leave redirected mounts/upper behind.
make_bad="$base/bad.json"
jq '.metadata.name="bad" | .command=["/not-an-executable"] | .args=[]' "$base/workspace.json" > "$make_bad"
bad_id="$(cri create "$sandbox" "$make_bad" "$base/pod.json")";containers+=("$bad_id")
if cri start "$bad_id" > "$artifacts/bad-start.txt" 2>&1; then echo 'invalid executable unexpectedly started'; exit 1; fi
cp "$base/state/$bad_id.json" "$artifacts/bad-wrapper.json"
test "$(jq -r .phase "$base/state/$bad_id.json")" = deleted
# Each actual normal delete must run wrapper -> real delete -> unmount -> cleanup.
printf '%s\n' 'Pod-scoped history survives sandbox deletion' > "$pod_log/ci-pod-marker"
for id in "$main_id" "$ttyd_id" "$init_id"; do
 cri stop "$id"; cri rm "$id"
 test "$(jq -r .phase "$base/state/$id.json")" = deleted
 test ! -e "$scratch/runtime-private/$id"
 ! grep -F "upperdir=$scratch/runtime-private/$id/upper" /proc/self/mountinfo
 done
cri rm "$bad_id"
cri stopp "$sandbox";cri rmp "$sandbox"
test "$(jq -r .phase "$base/state/$sandbox.json")" = deleted
! find "$scratch/runtime-private" -mindepth 1 -maxdepth 1 ! -name managed-files ! -name logs -print -quit | grep .
# A replacement sandbox with the same Pod UID must receive fresh kubelet hosts.
# Reuse must be scoped to a sandbox, because its Pod IP may have changed.
old_sandbox="$sandbox"
test -L "$pod_log"
cp "$pod_log/ci-pod-marker" "$artifacts/log-pod-marker-preserved-after-delete.txt"
printf '127.0.0.1 mwc-budget-new.local\n' >> "$kubelet_hosts"
sandbox="$(cri runp --runtime mwc-ci "$base/pod.json")"
containers+=("$sandbox")
test "$sandbox" != "$old_sandbox"
test "$(readlink "$pod_log")" = "$scratch/runtime-private/logs/$uid"
cmp "$pod_log/ci-pod-marker" "$artifacts/log-pod-marker-preserved-after-delete.txt"
make_container replacement idle
replacement_id="$last_id"
cri exec "$replacement_id" getent hosts mwc-budget-new.local > "$artifacts/replacement-sandbox-hosts.txt"
! grep -F mwc-budget-new.local "$scratch/runtime-private/managed-files/$old_sandbox/kubelet-hosts"
grep -F mwc-budget-new.local "$scratch/runtime-private/managed-files/$sandbox/kubelet-hosts"
cri stop "$replacement_id"; cri rm "$replacement_id"
cri stopp "$sandbox"; cri rmp "$sandbox"
test "$(jq -r .phase "$base/state/$replacement_id.json")" = deleted
test "$(jq -r .phase "$base/state/$sandbox.json")" = deleted
! grep -F "$scratch/runtime-private/managed-files/" /proc/self/mountinfo
# Reject nonempty node log directories and wrong links without moving/deleting
# existing data. A failed pause must never cause application log fallback.
for gate in nonempty wronglink; do
 gate_name="mwc-log-$gate"
 gate_log="$base/pod-logs/memeloop-workspace-control_${gate_name}_$uid"
 jq --arg name "$gate_name" --arg log "$gate_log" '.metadata.name=$name|.log_directory=$log' "$base/pod.json" > "$base/log-gate-pod.json"
 if [ "$gate" = nonempty ]; then mkdir "$gate_log"; echo keep > "$gate_log/keep"; else ln -s "$base" "$gate_log"; fi
 if cri runp --runtime mwc-ci "$base/log-gate-pod.json" > "$artifacts/log-gate-$gate.txt" 2>&1; then echo "log gate $gate unexpectedly started"; exit 1; fi
 if [ "$gate" = nonempty ]; then test "$(cat "$gate_log/keep")" = keep; rm "$gate_log/keep"; rmdir "$gate_log"; else test "$(readlink "$gate_log")" = "$base"; rm "$gate_log"; fi
done
# Inject the post-rmdir/pre-link interruption externally, then repeat kubelet's
# normal MkdirAll and retry. A correct link survives failures after link creation
# as the mount/unmount fault cases below separately prove.
gap_log="$base/pod-logs/memeloop-workspace-control_mwc-log-gap_$uid"
jq --arg log "$gap_log" '.metadata.name="mwc-log-gap"|.log_directory=$log' "$base/pod.json" > "$base/log-gap-pod.json"
mkdir "$gap_log"
if cri runp --runtime mwc-ci-fail-loglink "$base/log-gap-pod.json" > "$artifacts/log-gap-failed-runp.txt" 2>&1; then echo 'log-link interruption unexpectedly started'; exit 1; fi
grep -F 'EIO (Input/output error) (INJECTED)' "$artifacts/fault-loglink-syscalls.txt"*
test ! -e "$gap_log"; test ! -L "$gap_log"
mkdir -p "$gap_log"
gap_id="$(cri runp --runtime mwc-ci "$base/log-gap-pod.json")"; containers+=("$gap_id")
test "$(readlink "$gap_log")" = "$scratch/runtime-private/logs/$uid"
cri stopp "$gap_id"; cri rmp "$gap_id"
test -L "$gap_log"
# Inject failures at the actual switch syscalls through a separate traced CRI
# handler. The ordinary shim must remove even the restored original root mount.
for fault in mount unmount; do
 find "$base/state" -name '*.json' -printf '%f\n' | sort > "$base/states-before.txt"
 jq --arg name "mwc-budget-ci-fault-$fault" --arg log "$base/pod-logs/memeloop-workspace-control_mwc-budget-ci-fault-${fault}_$uid" '.metadata.name=$name | .log_directory=$log' "$base/pod.json" > "$base/fault-$fault-pod.json"
 mkdir -p "$(jq -r .log_directory "$base/fault-$fault-pod.json")"
 if cri runp --runtime "mwc-ci-fail-$fault" "$base/fault-$fault-pod.json" > "$artifacts/fault-$fault-runp.txt" 2>&1; then echo "injected $fault failure unexpectedly started sandbox"; exit 1; fi
 find "$base/state" -name '*.json' -printf '%f\n' | sort > "$base/states-after.txt"
 comm -13 "$base/states-before.txt" "$base/states-after.txt" > "$base/new-state.txt"
 test "$(wc -l < "$base/new-state.txt")" = 1
 fault_record="$(cat "$base/new-state.txt")"
 cp "$base/state/$fault_record" "$artifacts/fault-$fault-wrapper.json"
 fault_id="$(jq -r .id "$base/state/$fault_record")"
 ! grep -F "/$fault_id/rootfs " /proc/self/mountinfo
 ! grep -F "upperdir=$scratch/runtime-private/$fault_id/upper" /proc/self/mountinfo
 if [ "$fault" = mount ]; then
  grep -F 'EIO (Input/output error) (INJECTED)' "$artifacts/fault-$fault-syscalls.txt"*
  grep -F 'original cleanup mount restore: <nil>' "$base/state/errors.log"
 else
  grep -F 'EBUSY (Device or resource busy) (INJECTED)' "$artifacts/fault-$fault-syscalls.txt"*
 fi
 # The real shim's failed-create cleanup has normally unmounted the original,
 # then invoked runtime delete. Only this tested path permits ordinary cleanup;
 # arbitrary hard-kill/orphan recovery is not inferred from it.
 test "$(jq -r .phase "$base/state/$fault_id.json")" = deleted
 test ! -e "$scratch/runtime-private/$fault_id"
 test -L "$(jq -r .log_directory "$base/fault-$fault-pod.json")"
 jq -cn --arg fault "$fault" --arg id "$fault_id" '{gate:$fault,container_id:$id,rejected:true,rootfs_mount_leak:false,normal_shim_failure_cleanup_deleted_upper:true,production_orphan_gc_implemented:false}' >> "$artifacts/fault-results.jsonl"
done
source "$GITHUB_WORKSPACE/runtime-wrapper/ci/kubelet-logs.sh"
df -B1 "$scratch" "$home" > "$artifacts/after-container-delete-df.txt"
cp -a "$base/state" "$artifacts/final-wrapper-state"
printf '%s\n' 'native mechanism, shared capacity including managed /etc, Home/SSH/DNS, cleanup, identity/path/namespace negative gates, checkpoint rejection passed' > "$artifacts/acceptance.txt"
# Deliberately NOT accepted: image-defined/arbitrary writable host binds, runsc,
# changed kubelet accounting/eviction, production CSI crash/reboot/orphan lifecycle,
# or complete two-budget scope. Runsc has a separate isolated job and receipt.
