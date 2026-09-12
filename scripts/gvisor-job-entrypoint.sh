#!/usr/bin/env bash
# Submits a node transaction to host systemd and waits for its durable result.
set -euo pipefail

fail() { printf 'FAIL: %s\n' "$*" >&2; exit 1; }

expected_node=${EXPECTED_NODE:-}
transaction_id=${TRANSACTION_ID:-}
mode=${MODE:-install}
host_root=/host
payload_root=/opt/mwc
release='release-20260831.0'
checksum=014b3871a5c698c802fd7a03758e0dbf4c1683f9e3f8c743979ea66bbf6553a4

[[ $expected_node =~ ^[a-z0-9][a-z0-9.-]{0,62}$ ]] || fail 'EXPECTED_NODE is invalid'
[[ $transaction_id =~ ^[a-z0-9][a-z0-9-]{0,62}$ ]] || fail 'TRANSACTION_ID is invalid'
[[ $mode == install || $mode == rollback ]] || fail 'MODE must be install or rollback'
[[ -d $host_root && ! -L $host_root ]] || fail '/host is not a safe host-root mount'
[[ -r $host_root/etc/hostname ]] || fail 'host identity is unavailable'
host_name=$(tr -d ' \t\r\n' < "$host_root/etc/hostname")
[[ $host_name == "$expected_node" ]] \
  || fail "host name $host_name does not match EXPECTED_NODE $expected_node"

transaction_root="$host_root/var/lib/memeloop-workspace-control/node-maintenance/$transaction_id"
payload_manifest="$transaction_root/payload.sha256"
if [[ -e $transaction_root || -L $transaction_root ]]; then
  [[ -d $transaction_root && ! -L $transaction_root ]] || fail 'transaction path is unsafe'
  [[ -f $payload_manifest && ! -L $payload_manifest ]] || fail 'existing transaction has no immutable payload manifest'
  (cd "$transaction_root" && sha256sum --check --strict payload.sha256) \
    || fail 'existing transaction payload differs from its immutable manifest'
else
  staging="$host_root/var/lib/memeloop-workspace-control/node-maintenance/.${transaction_id}.new"
  [[ ! -e $staging && ! -L $staging ]] || fail 'transaction staging path already exists'
  install -d -m 0700 "$staging"
  for script in gvisor-host-transaction.sh gvisor-node-install.sh gvisor-node-preflight.sh gvisor-node-rollback.sh; do
    install -m 0755 "$payload_root/$script" "$staging/$script"
  done
  install -m 0644 "$payload_root/gvisor-x86_64.tar.bz2" "$staging/gvisor-x86_64.tar.bz2"
  printf '%s\n%s\n%s\n' "$expected_node" "$release" "$checksum" >"$staging/metadata"
  chmod 0600 "$staging/metadata"
  (cd "$staging" && sha256sum gvisor-host-transaction.sh gvisor-node-install.sh \
    gvisor-node-preflight.sh gvisor-node-rollback.sh gvisor-x86_64.tar.bz2 metadata \
    >payload.sha256)
  mv -T "$staging" "$transaction_root"
fi

printf '%s  %s\n' "$checksum" "$transaction_root/gvisor-x86_64.tar.bz2" | sha256sum --check --strict

recovery_unit="$host_root/etc/systemd/system/mwc-gvisor-recovery@.service"
if [[ -e $recovery_unit || -L $recovery_unit ]]; then
  [[ -f $recovery_unit && ! -L $recovery_unit ]] || fail 'host recovery unit path is unsafe'
  cmp -s "$payload_root/mwc-gvisor-recovery@.service" "$recovery_unit" \
    || fail 'host recovery unit differs from the managed definition'
else
  install -m 0644 "$payload_root/mwc-gvisor-recovery@.service" "$recovery_unit"
fi
nsenter --target 1 --mount --uts --ipc --net --pid --root=/proc/1/root --wd=/ -- \
  systemctl daemon-reload

host_transaction_root="/var/lib/memeloop-workspace-control/node-maintenance/$transaction_id"
result_file="$transaction_root/result-$mode"
attempt_file="$transaction_root/attempt-$mode"
log_file="$transaction_root/transaction-$mode.log"
unit="mwc-gvisor-${mode}-${transaction_id}.service"

show_log() {
  if [[ -f $log_file ]]; then
    tail -n 200 "$log_file" || true
  fi
}

baseline_attempt=0
[[ ! -f $attempt_file ]] || baseline_attempt=$(<"$attempt_file")
[[ $baseline_attempt =~ ^[0-9]+$ ]] || fail 'durable attempt counter is invalid'

unit_state=$(nsenter --target 1 --mount --uts --ipc --net --pid \
  --root=/proc/1/root --wd=/ -- \
  systemctl show "$unit" --property=LoadState --value 2>/dev/null || true)
if [[ $unit_state != loaded ]]; then
  nsenter --target 1 --mount --uts --ipc --net --pid \
    --root=/proc/1/root --wd=/ -- \
    systemd-run --unit "$unit" --property=Type=oneshot --property=TimeoutStartSec=12min \
      --property="OnFailure=mwc-gvisor-recovery@${transaction_id}.service" \
      /bin/bash "$host_transaction_root/gvisor-host-transaction.sh" \
      "$mode" "$expected_node" "$transaction_id" "$release" "$checksum"
else
  unit_active=$(nsenter --target 1 --mount --uts --ipc --net --pid \
    --root=/proc/1/root --wd=/ -- \
    systemctl show "$unit" --property=ActiveState --value 2>/dev/null || true)
  if [[ $unit_active != active && $unit_active != activating ]]; then
    nsenter --target 1 --mount --uts --ipc --net --pid \
      --root=/proc/1/root --wd=/ -- systemctl reset-failed "$unit" 2>/dev/null || true
    nsenter --target 1 --mount --uts --ipc --net --pid \
      --root=/proc/1/root --wd=/ -- systemctl start --no-block "$unit"
  fi
fi

for _ in $(seq 1 420); do
  current_attempt=0
  [[ ! -f $attempt_file ]] || current_attempt=$(<"$attempt_file")
  if [[ $current_attempt =~ ^[0-9]+$ ]] \
    && (( current_attempt > baseline_attempt )) \
    && [[ -f $result_file ]]; then
    result=$(<"$result_file")
    case "$result" in
      SUCCEEDED) show_log; printf 'PASS: host transaction succeeded\n'; exit 0 ;;
      ROLLED_BACK) show_log; fail 'installation failed and the host was rolled back' ;;
      RECOVERY_FAILED) show_log; fail 'host recovery failed; use the node console' ;;
      FAILED) show_log; fail 'host transaction failed before changing the runtime' ;;
    esac
  fi
  unit_active=$(nsenter --target 1 --mount --uts --ipc --net --pid \
    --root=/proc/1/root --wd=/ -- \
    systemctl show "$unit" --property=ActiveState --value 2>/dev/null || true)
  if [[ $unit_active == failed ]]; then
    show_log
    fail 'host transaction unit failed without a durable result'
  fi
  sleep 2
done
show_log
fail 'host transaction did not produce a final result within 14 minutes'
