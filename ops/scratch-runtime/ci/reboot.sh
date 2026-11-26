#!/usr/bin/env bash
# Disposable GitHub-hosted VM; test real systemd ordering across OS reboots.
set -euo pipefail
test "${GITHUB_ACTIONS:-}" = true
test "${RUNNER_ENVIRONMENT:-}" = github-hosted
umask 077
dir=$1
mode=${2:-ordinary}
mkdir -p "$dir"
src=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
fetch() {
  curl -fsSL --retry 2 -o "$dir/$1" "$2"
  printf '%s  %s\n' "$3" "$dir/$1" | sha256sum -c
}
fetch noble.img 'https://cloud-images.ubuntu.com/releases/24.04/release-20261017/ubuntu-24.04-server-cloudimg-amd64.img' \
  04e8f2461a16a44e78a370284471f29616cdd9baf1893df25807b71f985b6c94
fetch k3s 'https://github.com/k3s-io/k3s/releases/download/v1.36.2%2Bk3s1/k3s' \
  65a55ec56c24eab44383086166ec620a491952b7e23941a49ddca6e8a4c4b4de
disk_args=()
case "$mode" in
  ordinary) disk_size=16G; cpus=2; memory=4096 ;;
  workspace)
    disk_size=24G; cpus=3; memory=5120
    truncate -s 14G "$dir/home.raw"
    truncate -s 7G "$dir/scratch.raw"
    disk_args=(-drive "file=$dir/home.raw,format=raw,if=none,id=home,cache=none"
      -device virtio-blk-pci,drive=home,serial=mwc-ci-home
      -drive "file=$dir/scratch.raw,format=raw,if=none,id=scratch,cache=none"
      -device virtio-blk-pci,drive=scratch,serial=mwc-ci-scratch)
    ;;
  *) exit 2 ;;
esac
qemu-img resize "$dir/noble.img" "$disk_size"
ssh-keygen -q -t ed25519 -N '' -f "$dir/client"
ssh-keygen -q -t ed25519 -N '' -f "$dir/guest"
printf 'mwc-ci-guest %s\n' "$(cat "$dir/guest.pub")" > "$dir/known-hosts"
{
  printf '%s\n' '#cloud-config' 'ssh_pwauth: false' 'ssh_authorized_keys:'
  printf '  - %s\n' "$(cat "$dir/client.pub")"
  printf '%s\n' 'ssh_keys:' '  ed25519_private: |'
  sed 's/^/    /' "$dir/guest"
  printf '  ed25519_public: %s\n' "$(cat "$dir/guest.pub")"
} > "$dir/user-data"
printf '%s\n' 'instance-id: mwc-runtime-boot' 'local-hostname: mwc-runtime-boot' > "$dir/meta-data"
cloud-localds "$dir/seed.img" "$dir/user-data" "$dir/meta-data"
if [[ -c /dev/kvm && ! -w /dev/kvm ]]; then
  sudo chgrp "$(id -g)" /dev/kvm
  sudo chmod g+rw /dev/kvm
fi
if [[ -r /dev/kvm && -w /dev/kvm ]]; then
  acceleration=(-accel kvm -cpu host)
else
  acceleration=(-accel tcg,thread=multi -cpu max)
fi
printf 'Starting %s guest with %s, %s vCPUs, %s MiB RAM\n' "$mode" "${acceleration[1]}" "$cpus" "$memory"
# Explicit devices can be enumerated ahead of implicit virtio drives. Always
# boot the OS disk rather than a newly attached empty CSI disk or the seed.
qemu-system-x86_64 "${acceleration[@]}" -display none -smp "$cpus" -m "$memory" \
  -drive "file=$dir/noble.img,format=qcow2,if=none,id=os" -device virtio-blk-pci,drive=os,bootindex=1 \
  -drive "file=$dir/seed.img,format=raw,if=virtio,readonly=on" \
  "${disk_args[@]}" \
  -nic user,model=virtio,hostfwd=tcp:127.0.0.1:22345-:22 -serial "file:$dir/k3s-guest.log" &
vm_pid=$!
cleanup() {
  local status=$?
  if [[ $status != 0 && $mode == workspace ]]; then
    timeout 20s ssh "${options[@]}" -p 22345 ubuntu@127.0.0.1 \
      sudo bash /home/ubuntu/mwc-ci/workspace-reboot-guest.sh diagnostics || true
  fi
  kill "$vm_pid" 2>/dev/null || true
  wait "$vm_pid" 2>/dev/null || true
}
trap cleanup EXIT

options=(-o BatchMode=yes -o StrictHostKeyChecking=yes -o "UserKnownHostsFile=$dir/known-hosts"
  -o GlobalKnownHostsFile=/dev/null -o HostKeyAlias=mwc-ci-guest -o ConnectTimeout=2 -o ServerAliveInterval=5
  -o IdentitiesOnly=yes -i "$dir/client")
guest() { ssh "${options[@]}" -p 22345 ubuntu@127.0.0.1 "$@"; }
boot_id() {
  local prior=${1:-} id
  for _ in {1..120}; do
    if id=$(guest cat /proc/sys/kernel/random/boot_id 2>/dev/null) && [[ -n $id && $id != "$prior" ]]; then
      printf '%s\n' "$id"
      return
    fi
    kill -0 "$vm_pid"
    sleep 2
  done
  return 1
}
step() { guest sudo bash /home/ubuntu/mwc-ci/guest.sh "$@"; }
workspace_step() { guest sudo bash /home/ubuntu/mwc-ci/workspace-reboot-guest.sh "$@"; }
reboot_guest() {
  guest sudo systemd-run --quiet --on-active=2s /usr/bin/systemctl reboot --no-wall
  boot_id "$1"
}

first=$(boot_id)
guest 'cloud-init status --wait && mkdir -m 0755 ~/mwc-ci'
scp -q "${options[@]}" -P 22345 "$dir/k3s" "$src"/{mwc-scratch-mounts.sh,mwc-scratch-mounts.service,30-mwc-scratch-mounts.conf} \
  "$src/ci/guest.sh" ubuntu@127.0.0.1:mwc-ci/
if [[ $mode == workspace ]]; then
  fixture="$RUNNER_TEMP/product-storage-fixtures/mwc-native-buildkit-true.json"
  vm_tests="$GITHUB_WORKSPACE/tests/runtime-product-vm"
  (cd "$MWC_PRODUCT_STORAGE_INPUTS" && sha256sum -c SHA256SUMS)
  scp -q "${options[@]}" -P 22345 "$vm_tests/"{workspace-reboot-guest.sh,topolvm-values.yaml} "$fixture" \
    "$RUNNER_TEMP/product-storage-bin/"{mwc-runtime-wrapper,containerd-shim-cleanup} \
    "$MWC_PRODUCT_STORAGE_INPUTS/"{helm.tgz,topolvm.tgz,workspace-image.tar} \
    "$MWC_PRODUCT_STORAGE_INPUTS/registry/public-input-identity.json" ubuntu@127.0.0.1:mwc-ci/
  workspace_step install
fi
step install
step mounted ordinary-before-reboot | tee "$dir/boot-evidence.jsonl"
first_container=$(jq -er '.container_id' "$dir/boot-evidence.jsonl")
if [[ $mode == workspace ]]; then
  workspace_step seed >&2
  workspace_step observe > "$dir/workspace-before.json"
  workspace_step expand > "$dir/workspace-grow.json"
fi
second=$(reboot_guest "$first")
step mounted ordinary-before-reboot "$first_container" | tee -a "$dir/boot-evidence.jsonl"
if [[ $mode == workspace ]]; then
  workspace_step observe "$(jq -er .container_id "$dir/workspace-before.json")" > "$dir/workspace-after.json"
  jq -es --arg first "$first" --arg second "$second" '
    length == 2 and map(.boot_id) == [$first,$second] and .[0].pod_uid == .[1].pod_uid and
    .[0].container_id != .[1].container_id and .[0].volumes == .[1].volumes and
    all(.[]; .strict_ssh and .buildkit and (.volumes|length)==2 and
      (.cri_version | test("^v?2[.]3[.]2-k3s2-shim-cleanup$")))
  ' "$dir/workspace-before.json" "$dir/workspace-after.json"
  workspace_step reclaim > "$dir/workspace-reclaimed.json"
  workspace_step restore-daemon
fi
step rollback
third=$(reboot_guest "$second")
step unmounted ordinary-after-rollback | tee -a "$dir/boot-evidence.jsonl"
if [[ $mode == workspace ]]; then
  guest sudo k3s crictl version > "$dir/containerd-after-rollback.txt"
  grep -Eq '^RuntimeVersion:[[:space:]]+v?2\.3\.2-k3s2$' "$dir/containerd-after-rollback.txt"
fi
jq -es --arg first "$first" --arg second "$second" --arg third "$third" '
  length == 3 and map(.boot_id) == [$first,$second,$third] and
  (map(.boot_id) | unique | length) == 3 and map(.mount_state) == ["mounted","mounted","unmounted"] and
  .[0].pod_uid == .[1].pod_uid and .[0].container_id != .[1].container_id and .[1].pod_uid != .[2].pod_uid and
  all(.[]; .ordinary_log_and_exec and .original_log_inode_retained)
' "$dir/boot-evidence.jsonl"
