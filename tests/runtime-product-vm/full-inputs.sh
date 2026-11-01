#!/bin/bash
# CI stage only: unprivileged process; all disks are this job's ordinary files.
set -euo pipefail
: "${MWC_VM_JOB_UUID:?}"
cd /work
mkdir -p output input/{compiled,debs,apt-lists,registry} stage
cp -a /compiled/. input/compiled/
(cd input/compiled && sha256sum -c SHA256SUMS)
cp input/compiled/source-shas.json output/compiled-source-shas.json
cp input/compiled/SHA256SUMS output/compiled-SHA256SUMS
cp input/compiled/artifact-identity.json output/compiled-artifact-identity.json
jq -e '.renderer_sha=="7a0d43abaa4d95ad2ecff55ebba1abc70b754564" and .wrapper_sha=="9f28c1fcad70e8b538caa1ef848b66dd87aed3ef"' input/compiled/source-shas.json
cp /app/runtime_product_storage.sh input/
cp /app/vm/guest-full.sh input/
cp /app/vm/inputs.json output/input-pins.json
curl -fSL --connect-timeout 15 --max-time 600 "$(jq -r .guest_url /app/vm/inputs.json)" -o guest-base.qcow2
printf '%s  %s\n' "$(jq -r .guest_sha512 /app/vm/inputs.json)" guest-base.qcow2 | sha512sum -c -
fetch() { curl -fSL --connect-timeout 15 --max-time 600 "$2" -o "input/$1"; printf '%s  %s\n' "$3" "input/$1" | sha256sum -c -; }
fetch k3s 'https://github.com/k3s-io/k3s/releases/download/v1.36.2%2Bk3s1/k3s' 65a55ec56c24eab44383086166ec620a491952b7e23941a49ddca6e8a4c4b4de
fetch helm.tgz https://get.helm.sh/helm-v4.2.4-linux-amd64.tar.gz c306b46f719b0a4da32d0f78ee21bf90ce8d602f15b22ab753f0674d1670a7f3
fetch topolvm.tgz https://github.com/topolvm/topolvm/releases/download/topolvm-chart-v17.2.0/topolvm-17.2.0.tgz 4b6c070da0b38f098c9ef6b6a38ef4e2dfff52adecedc280ad92bfab408f90ee
# Exact guest package closure is prepared by the CI image build. This is data,
# not host installation, and includes the matching snapshot's package indexes.
cp -a /app/debs/. input/debs/
cp -a /app/apt-lists/. input/apt-lists/
chmod 0755 input/k3s
K3S_DATA_DIR=/work/stage/k3s input/k3s ctr version --help >/dev/null
export PATH="/work/stage/k3s/data/current/bin:$PATH"
cat > stage/containerd.toml <<'TOML'
version = 3
root = '/work/stage/content'
state = '/work/stage/state'
disabled_plugins = ['io.containerd.cri.v1.images', 'io.containerd.cri.v1.runtime']
[grpc]
  address = '/work/stage/containerd.sock'
TOML
containerd --config stage/containerd.toml > output/stage-containerd.log 2>&1 & containerd_pid=$!
trap 'kill "$containerd_pid" 2>/dev/null || true; wait "$containerd_pid" || true' EXIT
ctr_stage() { ctr --address /work/stage/containerd.sock --namespace k8s.io "$@"; }
for attempt in $(seq 1 30); do if ctr_stage version >/dev/null 2>&1; then break; fi; sleep 1; done
mapfile -t images < <(jq -r '[.pod.spec.containers[],.pod.spec.initContainers[]?]|.[].image' input/compiled/fixtures/*.json | sort -u)
# The chart uses its one actual image for all sidecars. These two control-plane
# images correspond to the exact k3s release and explicit sandbox config.
images+=(ghcr.io/topolvm/topolvm-with-sidecar@sha256:70548dbe0c6addcccf79a557f29e95db2e6dc2cba2102988c91f30086004d0fc registry.k8s.io/pause:3.10 docker.io/rancher/mirrored-coredns-coredns:1.14.4)
for image in "${images[@]}"; do timeout 1200 ctr_stage images pull --skip-unpack --platform linux/amd64 "$image"; done
ctr_stage images ls > output/staged-images.txt
# Bound real unpack allocation from each unique actual layer, including block
# rounding and inode metadata; removed/overwritten files stay in this bound.
: > output/layer-digests.txt
for image in "${images[@]}"; do
 root="$(ctr_stage images ls | awk -v ref="$image" '$1==ref{print $3}')"
 ctr_stage content get "$root" > stage/root.json
 if jq -e '.manifests' stage/root.json >/dev/null; then
  root="$(jq -er '.manifests[]|select(.platform.os=="linux" and .platform.architecture=="amd64")|.digest' stage/root.json)"
  ctr_stage content get "$root" > stage/root.json
 fi
 jq -er '.layers[].digest' stage/root.json >> output/layer-digests.txt
 # Reject another compression codec explicitly instead of undercounting it.
 jq -e 'all(.layers[];.mediaType|endswith("tar+gzip") or endswith("tar.gzip"))' stage/root.json >/dev/null
 done
sort -u output/layer-digests.txt -o output/layer-digests.txt
: > output/layer-unpack-bound.tsv
while read -r digest; do
 bytes="$(ctr_stage content get "$digest" | gzip -dc | tar -tvf - | awk '
  $1 ~ /^-/ { bytes += int(($3+4095)/4096)*4096+1024 }
  $1 ~ /^d/ { bytes += 5120 }
  END { printf "%.0f\n", bytes }')"
 printf '%s\t%s\n' "$digest" "$bytes" >> output/layer-unpack-bound.tsv
done < output/layer-digests.txt
unpack_bound="$(awk '{n+=$2}END{printf "%.0f\n",n}' output/layer-unpack-bound.tsv)"
jq -n --rawfile inventory output/staged-images.txt '{actual_containerd_image_inventory:$inventory}' > input/registry/staged.json
# One OCI archive deduplicates shared blobs and preserves original image names.
ctr_stage images export --platform linux/amd64 --skip-manifest-json input/images.tar "${images[@]}"
archive_bytes="$(stat -c %s input/images.tar)"
kill "$containerd_pid"; wait "$containerd_pid" || true
trap - EXIT
rm -rf stage
(cd input && find . -type f ! -name SHA256SUMS -print0 | sort -z | xargs -0 sha256sum > SHA256SUMS)
# mke2fs -d requires no mount, loop device, root or host namespace. The guest
# deletes only the image archive after import and fstrim releases its duplicate.
truncate -s 4G inputs.raw
mke2fs -q -t ext4 -F -m 0 -L MWCINPUT -d input inputs.raw
rm -rf input
mkdir seed
printf 'instance-id: mwc-full-%s\nlocal-hostname: mwc-product-csi\n' "$MWC_VM_JOB_UUID" > seed/meta-data
cat > seed/user-data <<'YAML'
#cloud-config
users: []
ssh_pwauth: false
disable_root: true
package_update: false
package_upgrade: false
runcmd:
  - [mkdir, -p, /mnt/inputs]
  - [mount, /dev/disk/by-label/MWCINPUT, /mnt/inputs]
  - [bash, /mnt/inputs/guest-full.sh]
YAML
genisoimage -quiet -r -J -V cidata -o seed.iso seed
qemu-img create -f qcow2 -F qcow2 -b /work/guest-base.qcow2 guest-overlay.qcow2 32G
du -sk guest-base.qcow2 inputs.raw guest-overlay.qcow2 > output/full-stage-disk-kib.txt
# During import the second content copy exists; after the input disk's actual
# trim it is replaced by unpacked layers. Keep the same one-GiB shared reserve.
boot_and_guest_writes=805306368
scratch_fill=1073741824
home_metadata=268435456
qcow_metadata_and_artifacts=268435456
shared_reserve=1073741824
import_increment=$((archive_bytes+boot_and_guest_writes))
after_trim_increment=$((unpack_bound+boot_and_guest_writes+scratch_fill+home_metadata+qcow_metadata_and_artifacts))
if [ "$import_increment" -gt "$after_trim_increment" ]; then vm_increment="$import_increment"; else vm_increment="$after_trim_increment"; fi
actual_free="$(df -B1 --output=avail /work | tail -n1 | tr -d ' ')"
jq -n --argjson free "$actual_free" --argjson archive "$archive_bytes" --argjson unpack "$unpack_bound" \
 --argjson increment "$vm_increment" --argjson reserve "$shared_reserve" \
 '{actual_free_before_vm:$free,image_archive_bytes:$archive,actual_unique_layer_unpack_bound:$unpack,maximum_additional_vm_bytes:$increment,shared_reserve_bytes:$reserve,capacity_gate_passed:($free>=($increment+$reserve))}' > output/full-budget.json
jq -e '.capacity_gate_passed==true' output/full-budget.json
set +e
timeout --signal=TERM --kill-after=20s 5400s qemu-system-x86_64 \
 -accel tcg -machine q35 -cpu max -smp 2 -m 6144 \
 -nodefaults -display none -monitor none -serial stdio \
 -sandbox on,obsolete=deny,elevateprivileges=deny,spawn=deny,resourcecontrol=deny \
 -drive file=/work/guest-overlay.qcow2,format=qcow2,if=virtio,discard=unmap \
 -drive file=/work/inputs.raw,format=raw,if=virtio,discard=unmap,detect-zeroes=unmap \
 -drive file=/work/seed.iso,format=raw,media=cdrom,readonly=on \
 -device virtio-net-pci,netdev=net0 -netdev user,id=net0,restrict=on \
 2>&1 | head -c 33554432 > output/serial.log
result="${PIPESTATUS[0]}"
set -e
printf '%s\n' "$result" > output/qemu.exit
du -sk guest-base.qcow2 inputs.raw guest-overlay.qcow2 > output/full-final-disk-kib.txt
tr -d '\r' < output/serial.log > output/serial-clean.log
sed -n '/^MWC_FULL_ARTIFACT_BEGIN$/,/^MWC_FULL_ARTIFACT_END$/p' output/serial-clean.log | sed '1d;$d' | base64 -d > output/guest-evidence.tgz
sed -n 's/^MWC_VM_FULL_RECEIPT //p' output/serial-clean.log | tail -n1 > output/guest-receipt.json
rm output/serial-clean.log
test "$result" = 0
jq -e '.accepted==true and .full_harness_tested==true and .production_deployed==false' output/guest-receipt.json > output/acceptance.json
