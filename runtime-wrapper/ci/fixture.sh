#!/bin/bash
# Disposable privileged GitHub VM only. Never invoke against a cluster/node.
set -euo pipefail
umask 022
: "${GITHUB_ACTIONS:?This fixture only runs on a disposable GitHub Actions VM}"
: "${RUNNER_TEMP:?}"
base="/var/lib/mwc-runtime-prototype"
artifacts="$GITHUB_WORKSPACE/runtime-wrapper/artifacts"
mkdir -p "$base" "$artifacts"
install -m 0755 "$RUNNER_TEMP/mwc-runtime-prototype/mwc-runtime-wrapper" "$base/mwc-runtime-wrapper"
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
sandbox=''
containers=()
loops=()
cleanup() {
 set +e
 for id in "${containers[@]}"; do cri stop "$id"; cri rm "$id"; done
 if [ -n "$sandbox" ]; then cri stopp "$sandbox"; cri rmp "$sandbox"; fi
 cat /proc/self/mountinfo > "$artifacts/final-mountinfo.txt"
 if [ -n "$containerd_pid" ]; then kill "$containerd_pid"; wait "$containerd_pid"; fi
 # Normal unmounts only. EBUSY is evidence/failure, never hidden by lazy/force.
 umount "$scratch"; umount "$home"
 vgremove -fy "$vg"; vgremove -fy "$home_vg"
 for loop in "${loops[@]}"; do losetup -d "$loop"; done
}
trap cleanup EXIT
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
mkdir -p "$base/containerd-root/io.containerd.snapshotter.v1.overlayfs/snapshots" "$base/containerd-state/io.containerd.runtime.v2.task/k8s.io" "$base/state" "$base/kubelet"
chmod 0700 "$base/state"
vg_uuid="$(vgs --noheadings -o vg_uuid "$vg" | tr -d ' -\n')"
pool_uuid="$(dmsetup info -c --noheadings -o uuid "$vg-pool-tpool" | xargs)"
real_runc="$(readlink -f "$(command -v runc)")"
jq -n --arg runtime "$real_runc" --arg kubelet "$base/kubelet" --arg bundle "$base/containerd-state/io.containerd.runtime.v2.task/k8s.io" --arg snapshot "$base/containerd-root/io.containerd.snapshotter.v1.overlayfs/snapshots" --arg state "$base/state" --arg namespace "/proc/$$/ns/mnt" --arg vg "$vg_uuid" --arg pool "$pool_uuid" --arg dmsetup "$(readlink -f "$(command -v dmsetup)")" '{real_runtime:$runtime,kubelet_root:$kubelet,bundle_root:$bundle,snapshot_root:$snapshot,state_root:$state,mount_namespace:$namespace,scratch_vg_uuid:$vg,scratch_pool_dm_uuid:$pool,dmsetup:$dmsetup}' > /etc/mwc-runtime-wrapper.json
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
containerd --config "$base/containerd.toml" > "$artifacts/containerd.log" 2>&1 &
containerd_pid=$!
for n in $(seq 1 60); do if cri info > "$artifacts/cri-info.json"; then break; fi; sleep 1; done
cri info > "$artifacts/cri-info.json"
ctr_ci images import "$base/image.tar"
cri pull registry.k8s.io/pause:3.10
jq -n --arg uid "$uid" --arg log "$base/pod-logs" '{metadata:{name:"mwc-budget-ci",namespace:"mwc-workspaces",uid:$uid,attempt:0},log_directory:$log,linux:{security_context:{namespace_options:{network:2}}}}' > "$base/pod.json"
mkdir -p "$base/pod-logs"
sandbox="$(cri runp --runtime mwc-ci "$base/pod.json")"
containers+=("$sandbox")

make_container() {
 local name="$1"; shift
 jq -n --arg name "$name" --arg home "$home" --arg data "$scratch/data" --args '{metadata:{name:$name,attempt:0},image:{image:"docker.io/library/mwc-runtime-fixture:ci"},command:["/usr/local/bin/mwc-budget-workload"],args:$ARGS.positional,log_path:($name+".log"),mounts:[{container_path:"/home/user",host_path:$home},{container_path:"/scratch",host_path:$data},{container_path:"/tmp",host_path:($data+"/tmp")},{container_path:"/var/tmp",host_path:($data+"/tmp")}],linux:{security_context:{capabilities:{drop_capabilities:["SYS_ADMIN"]},no_new_privs:false}}}' "$@" > "$base/$name.json"
 local id
 id="$(cri create "$sandbox" "$base/$name.json" "$base/pod.json")"
 containers+=("$id"); last_id="$id"
 cp "$base/containerd-state/io.containerd.runtime.v2.task/k8s.io/$id/config.json" "$artifacts/$name-oci.json" 2>/dev/null || true
 cri start "$id"
}
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
cat /proc/self/mountinfo > "$artifacts/active-mountinfo.txt"
ssh_ci() { ssh -i "$base/client" -p 2222 -o BatchMode=yes -o StrictHostKeyChecking=yes -o UserKnownHostsFile="$base/known_hosts" -o ConnectTimeout=5 user@127.0.0.1 "$@"; }
for n in $(seq 1 20); do if ssh_ci 'id'; then break; fi; sleep 1; done
ssh_ci 'sudo -n id -u; test ! -e /scratch/runtime-private; test ! -e /runtime-private; touch /home/user/ssh-marker' | tee "$artifacts/ssh-before-full.txt"
# Copy-up from immutable image, multi UID, and sidecar allocation consume same FS.
cri exec "$main_id" bash -c 'printf changed >> /seed-copy-up; sudo -u user touch /home/user/multi-uid; dd if=/dev/zero of=/scratch/data-initial bs=1M count=32 conv=fsync'
cri exec "$ttyd_id" bash -c 'dd if=/dev/zero of=/sidecar-allocation bs=1M count=16 conv=fsync'
df -B1 "$scratch" "$home" > "$artifacts/before-full-df.txt"
cri exec "$main_id" /usr/local/bin/mwc-budget-workload fill | tee "$artifacts/full-result.txt"
ssh_ci 'test -f /home/user/home-still-writable && sudo -n sh -c "echo ssh-alive > /home/user/after-full"' > "$artifacts/ssh-after-full.txt"
cri inspect "$main_id" > "$artifacts/main-after-full.json"
test "$(jq -r .status.state "$artifacts/main-after-full.json")" = CONTAINER_RUNNING
cri exec "$main_id" bash -c 'rm /rootfs-fill /scratch/data-fill /scratch/data-initial; echo recovered > /rootfs-after-free; sync'
df -B1 "$scratch" "$home" > "$artifacts/after-delete-df.txt"
# Invalid real-runc create must not leave redirected mounts/upper behind.
make_bad="$base/bad.json"
jq '.metadata.name="bad" | .command=["/not-an-executable"] | .args=[]' "$base/workspace.json" > "$make_bad"
bad_id="$(cri create "$sandbox" "$make_bad" "$base/pod.json")";containers+=("$bad_id")
if cri start "$bad_id" > "$artifacts/bad-start.txt" 2>&1; then echo 'invalid executable unexpectedly started'; exit 1; fi
cp "$base/state/$bad_id.json" "$artifacts/bad-wrapper.json"
test "$(jq -r .phase "$base/state/$bad_id.json")" = deleted
# Each actual normal delete must run wrapper -> real delete -> unmount -> cleanup.
for id in "$main_id" "$ttyd_id" "$init_id"; do
 cri stop "$id"; cri rm "$id"
 test "$(jq -r .phase "$base/state/$id.json")" = deleted
 test ! -e "$scratch/runtime-private/$id"
 ! grep -F "upperdir=$scratch/runtime-private/$id/upper" /proc/self/mountinfo
 done
cri rm "$bad_id"
cri stopp "$sandbox";cri rmp "$sandbox"
test "$(jq -r .phase "$base/state/$sandbox.json")" = deleted
! find "$scratch/runtime-private" -mindepth 1 -print -quit | grep .
cp -a "$base/state" "$artifacts/final-wrapper-state"
printf '%s\n' 'native mechanism, shared-rootfs/data capacity, Home independence, strict SSH, normal cleanup, failed executable cleanup passed' > "$artifacts/acceptance.txt"
# Deliberately NOT accepted: logs, arbitrary writable host binds, runsc, kubelet
# eviction/statistics, crash/reboot/orphan lifecycle, or complete two-budget scope.
