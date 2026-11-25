#!/bin/bash
# Executed by cloud-init inside the disposable VM, never on the CI node.
set -euo pipefail
exec > /dev/ttyS0 2>&1
base=/var/lib/mwc-vm-import-smoke
mkdir -p "$base"
containerd_pid=''
finish() {
 result=$?
 trap - EXIT
 if [ -n "$containerd_pid" ]; then kill "$containerd_pid"; wait "$containerd_pid" || true; fi
 if [ "$result" = 0 ]; then accepted=true; else accepted=false; fi
 printf '\nMWC_VM_SMOKE_RECEIPT {"accepted":%s,"exit_status":%s,"actual_guest_boot":true,"exact_rust_manifest":"sha256:d50798f0f8345d7295c303ec0f04b1f1e064b7d6ac9b074d87065a488957360f","actual_containerd_import":%s,"layers_unpacked":false,"k3s_server_started":false,"real_csi_tested":false,"full_harness_tested":false}\n' "$accepted" "$result" "$accepted"
 sync
 /sbin/poweroff
}
trap finish EXIT
uname -a
cat /proc/cmdline
findmnt /
findmnt /mnt/inputs
sha256sum --check /mnt/inputs/k3s.sha256
install -m 0755 /mnt/inputs/k3s "$base/k3s"
K3S_DATA_DIR="$base/k3s-data" "$base/k3s" ctr version --help >/dev/null
export PATH="$base/k3s-data/data/current/bin:$PATH"
containerd --version | tee "$base/containerd-version.txt"
runc --version | tee "$base/runc-version.txt"
grep -F 'v2.3.2-k3s2' "$base/containerd-version.txt"
grep -Fx 'runc version 1.4.2' "$base/runc-version.txt"
cat > "$base/containerd.toml" <<TOML
version = 3
root = '$base/content'
state = '$base/state'
disabled_plugins = ['io.containerd.cri.v1.images', 'io.containerd.cri.v1.runtime']
[grpc]
  address = '$base/containerd.sock'
TOML
containerd --config "$base/containerd.toml" > "$base/containerd.log" 2>&1 & containerd_pid=$!
for attempt in $(seq 1 30); do
 if ctr --address "$base/containerd.sock" version >/dev/null 2>&1; then break; fi
 sleep 1
done
# Streaming tar avoids a second 1.7GB archive in the guest. Containerd verifies
# content digests during import; no image process is run in this smoke slice.
tar -C /mnt/inputs/rust-oci -cf - . | ctr --address "$base/containerd.sock" --namespace k8s.io images import --no-unpack --digests -
ctr --address "$base/containerd.sock" --namespace k8s.io content get sha256:d50798f0f8345d7295c303ec0f04b1f1e064b7d6ac9b074d87065a488957360f > "$base/manifest.json"
printf '%s  %s\n' d50798f0f8345d7295c303ec0f04b1f1e064b7d6ac9b074d87065a488957360f "$base/manifest.json" | sha256sum -c -
ctr --address "$base/containerd.sock" --namespace k8s.io content get sha256:fc173cab292b6b7f9ac12db3912ca1c280070bfe9e3e7dcb518caefa2fa9fab4 > "$base/config.json"
printf '%s  %s\n' fc173cab292b6b7f9ac12db3912ca1c280070bfe9e3e7dcb518caefa2fa9fab4 "$base/config.json" | sha256sum -c -
ctr --address "$base/containerd.sock" --namespace k8s.io images ls
ctr --address "$base/containerd.sock" --namespace k8s.io content ls
ctr --address "$base/containerd.sock" --namespace k8s.io snapshots ls
! ctr --address "$base/containerd.sock" --namespace k8s.io snapshots ls -q | grep .
df -B1 /
du -sk "$base/content"
