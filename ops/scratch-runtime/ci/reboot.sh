#!/usr/bin/env bash
# Disposable GitHub-hosted VM; test real systemd ordering across OS reboots.
set -euo pipefail
test "${GITHUB_ACTIONS:-}" = true
test "${RUNNER_ENVIRONMENT:-}" = github-hosted
umask 077
dir=$1
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
qemu-img resize "$dir/noble.img" 16G
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
qemu-system-x86_64 "${acceleration[@]}" -display none -smp 2 -m 4096 \
  -drive "file=$dir/noble.img,format=qcow2,if=virtio" -drive "file=$dir/seed.img,format=raw,if=virtio,readonly=on" \
  -nic user,model=virtio,hostfwd=tcp:127.0.0.1:22345-:22 -serial "file:$dir/k3s-guest.log" &
vm_pid=$!
cleanup() { kill "$vm_pid" 2>/dev/null || true; wait "$vm_pid" 2>/dev/null || true; }
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
reboot_guest() {
  guest sudo systemd-run --quiet --on-active=2s /usr/bin/systemctl reboot --no-wall
  boot_id "$1"
}

first=$(boot_id)
guest 'cloud-init status --wait && mkdir -m 0755 ~/mwc-ci'
scp -q "${options[@]}" -P 22345 "$dir/k3s" "$src"/{mwc-scratch-mounts.sh,mwc-scratch-mounts.service,30-mwc-scratch-mounts.conf} \
  "$src/ci/guest.sh" ubuntu@127.0.0.1:mwc-ci/
step install
step mounted ordinary-before-reboot | tee "$dir/boot-evidence.jsonl"
first_container=$(jq -er '.container_id' "$dir/boot-evidence.jsonl")
second=$(reboot_guest "$first")
step mounted ordinary-before-reboot "$first_container" | tee -a "$dir/boot-evidence.jsonl"
step rollback
third=$(reboot_guest "$second")
step unmounted ordinary-after-rollback | tee -a "$dir/boot-evidence.jsonl"
jq -es --arg first "$first" --arg second "$second" --arg third "$third" '
  length == 3 and map(.boot_id) == [$first,$second,$third] and
  (map(.boot_id) | unique | length) == 3 and map(.mount_state) == ["mounted","mounted","unmounted"] and
  .[0].pod_uid == .[1].pod_uid and .[0].container_id != .[1].container_id and .[1].pod_uid != .[2].pod_uid and
  all(.[]; .ordinary_log_and_exec and .original_log_inode_retained)
' "$dir/boot-evidence.jsonl"
