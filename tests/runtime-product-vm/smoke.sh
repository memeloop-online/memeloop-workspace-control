#!/bin/bash
# Only launched by the reviewed Forgejo launcher, in a non-privileged container.
set -euo pipefail
: "${MWC_VM_JOB_UUID:?}"
[[ "$MWC_VM_JOB_UUID" =~ ^[0-9a-f-]{36}$ ]]
cd /work
mkdir -p output input
qemu-system-x86_64 --version > output/qemu-version.txt
skopeo --version > output/skopeo-version.txt
cp /app/vm/inputs.json output/input-pins.json
jq '.smoke_budget_bytes' /app/vm/inputs.json > output/budget.json
# Peak inputs are not all simultaneous: the ISO replaces its staging tree before
# VM boot. Disk headroom is measured by the launcher on the actual graph volume.
guest_url="$(jq -r .guest_url /app/vm/inputs.json)"
guest_hash="$(jq -r .guest_sha512 /app/vm/inputs.json)"
curl -fSL --connect-timeout 15 --max-time 600 "$guest_url" -o guest-base.qcow2
printf '%s  %s\n' "$guest_hash" guest-base.qcow2 | sha512sum -c -
k3s_url="$(jq -r .k3s_url /app/vm/inputs.json)"
k3s_hash="$(jq -r .k3s_sha256 /app/vm/inputs.json)"
curl -fSL --connect-timeout 15 --max-time 600 "$k3s_url" -o input/k3s
printf '%s  %s\n' "$k3s_hash" input/k3s | sha256sum -c -
printf '%s  %s\n' "$k3s_hash" /mnt/inputs/k3s > input/k3s.sha256
skopeo copy --preserve-digests --override-os linux --override-arch amd64 \
 "docker://$(jq -r .rust_image /app/vm/inputs.json)" oci:input/rust-oci:rust-d507
manifest_hash="$(jq -r .rust_manifest_sha256 /app/vm/inputs.json)"
printf '%s  %s\n' "$manifest_hash" "input/rust-oci/blobs/sha256/$manifest_hash" | sha256sum -c -
cp /app/vm/guest-smoke.sh input/guest-smoke.sh
# Retain the exact existing full harness as a future input, not a duplicated
# implementation. The smoke does not invoke it or claim CSI acceptance.
cp /app/runtime_product_storage.sh input/runtime_product_storage.sh
cp /app/vm/inputs.json input/inputs.json
genisoimage -quiet -r -J -V MWCINPUT -o inputs.iso input
rm -rf input
mkdir seed
printf 'instance-id: mwc-vm-%s\nlocal-hostname: mwc-import-smoke\n' "$MWC_VM_JOB_UUID" > seed/meta-data
cat > seed/user-data <<'YAML'
#cloud-config
users: []
ssh_pwauth: false
disable_root: true
package_update: false
package_upgrade: false
runcmd:
  - [mkdir, -p, /mnt/inputs]
  - [mount, -o, ro, /dev/disk/by-label/MWCINPUT, /mnt/inputs]
  - [bash, /mnt/inputs/guest-smoke.sh]
YAML
genisoimage -quiet -r -J -V cidata -o seed.iso seed
qemu-img create -f qcow2 -F qcow2 -b /work/guest-base.qcow2 guest-overlay.qcow2 8G
qemu-img info --output=json guest-overlay.qcow2 > output/guest-overlay-before.json
# No /dev/kvm, tap, host namespace, host block device, listener or guest SSH key.
# user-mode NAT is restricted; every required byte is on read-only virtio media.
set +e
timeout --signal=TERM --kill-after=20s 2700s qemu-system-x86_64 \
 -accel tcg -machine q35 -cpu max -smp 2 -m 2048 \
 -nodefaults -display none -monitor none -serial stdio \
 -sandbox on,obsolete=deny,elevateprivileges=deny,spawn=deny,resourcecontrol=deny \
 -drive file=/work/guest-overlay.qcow2,format=qcow2,if=virtio \
 -drive file=/work/inputs.iso,format=raw,media=cdrom,readonly=on \
 -drive file=/work/seed.iso,format=raw,media=cdrom,readonly=on \
 -device virtio-net-pci,netdev=net0 -netdev user,id=net0,restrict=on \
 2>&1 | head -c 33554432 > output/serial.log
result="${PIPESTATUS[0]}"
set -e
printf '%s\n' "$result" > output/qemu.exit
qemu-img info --output=json guest-overlay.qcow2 > output/guest-overlay-after.json
du -sk guest-base.qcow2 inputs.iso guest-overlay.qcow2 > output/actual-disk-kib.txt
# Serial and QEMU stderr share the capped log above, including boot failures.
# Preserve only finite logs/manifests, never multi-GB guest disks as artifacts.
tr -d '\r' < output/serial.log | sed -n 's/^MWC_VM_SMOKE_RECEIPT //p' | tail -n1 > output/guest-receipt.json
test "$result" = 0
jq -e '.accepted==true and .actual_guest_boot==true and .actual_containerd_import==true and .layers_unpacked==false and .real_csi_tested==false' output/guest-receipt.json > output/acceptance.json
