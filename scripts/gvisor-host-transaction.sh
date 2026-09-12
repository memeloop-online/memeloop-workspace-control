#!/usr/bin/env bash
# Runs as a host systemd oneshot. Host systemd can recover it across a K3s restart.
set -euo pipefail

fail() { printf 'FAIL: %s\n' "$*" >&2; exit 1; }

base=/var/lib/memeloop-workspace-control/node-maintenance
mode=${1:-}
if [[ $mode == recover ]]; then
  transaction_id=${2:-}
  [[ $transaction_id =~ ^[a-z0-9][a-z0-9-]{0,62}$ ]] || fail 'invalid transaction ID'
  transaction_root="$base/$transaction_id"
  [[ -f $transaction_root/metadata && ! -L $transaction_root/metadata ]] || fail 'recovery metadata is unavailable'
  mapfile -t metadata <"$transaction_root/metadata"
  [[ ${#metadata[@]} == 4 ]] || fail 'recovery metadata is invalid'
  expected_node=${metadata[0]}
  expected_host=${metadata[1]}
  release=${metadata[2]}
  checksum=${metadata[3]}
  result_mode=install
else
  expected_node=${2:-}
  transaction_id=${3:-}
  release=${4:-}
  checksum=${5:-}
  expected_host=${6:-$expected_node}
  transaction_root="$base/$transaction_id"
  result_mode=$mode
fi

[[ $mode == install || $mode == rollback || $mode == recover ]] || fail 'invalid transaction mode'
[[ $expected_node =~ ^[a-z0-9][a-z0-9.-]{0,62}$ ]] || fail 'invalid node name'
[[ $expected_host =~ ^[a-z0-9][a-z0-9.-]{0,62}$ ]] || fail 'invalid host name'
[[ $transaction_id =~ ^[a-z0-9][a-z0-9-]{0,62}$ ]] || fail 'invalid transaction ID'
[[ $checksum =~ ^[a-f0-9]{64}$ ]] || fail 'invalid archive checksum'
IFS= read -r host_name </proc/sys/kernel/hostname
[[ $host_name == "$expected_host" ]] || fail 'host name does not match the approved node mapping'
[[ -d $transaction_root && ! -L $transaction_root ]] || fail 'transaction directory is unsafe'

result_file="$transaction_root/result-$result_mode"
state_file="$transaction_root/state-$result_mode"
attempt_file="$transaction_root/attempt-$result_mode"
log_file="$transaction_root/transaction-$mode.log"
template=/var/lib/rancher/k3s/agent/etc/containerd/config-v3.toml.tmpl
rendered=/var/lib/rancher/k3s/agent/etc/containerd/config.toml
version_dir="/usr/local/lib/gvisor/$checksum"
runsc_config="$version_dir/runsc.toml"
runsc_link=/usr/local/bin/runsc
shim_link=/usr/local/bin/containerd-shim-runsc-v1

exec >>"$log_file" 2>&1
export PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
unset GVISOR_NODE_TEST_MODE GVISOR_NODE_TEST_ROOT GVISOR_NODE_TEST_HOSTNAME \
  GVISOR_NODE_TEST_ARCH GVISOR_NODE_TEST_KERNEL GVISOR_NODE_TEST_CGROUP_FS \
  GVISOR_NODE_TEST_SYSTEMD_VERSION GVISOR_NODE_TEST_SERVICE GVISOR_NODE_TEST_INIT

write_file() {
  local path=$1 value=$2 temporary
  temporary=$(mktemp "$transaction_root/.state.XXXXXX")
  printf '%s\n' "$value" >"$temporary"
  chmod 0600 "$temporary"
  mv -T "$temporary" "$path"
}
state() {
  write_file "$state_file" "$1"
  printf '%s state=%s node=%s transaction=%s release=%s\n' \
    "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$1" "$expected_node" "$transaction_id" "$release"
}
finalized=false
phase=PRECHECK
finish() {
  state "$1"
  write_file "$result_file" "$1"
  finalized=true
}
# shellcheck disable=SC2317 # Invoked by the EXIT trap.
on_exit() {
  local status=$?
  if [[ $finalized != true && $phase == PRECHECK ]]; then
    state FAILED || true
    write_file "$result_file" FAILED || true
  fi
  exit "$status"
}
install -d -m 0700 "$base"
exec 9>"$base/transaction.lock"
flock -n 9 || fail 'another gVisor node transaction is active'
trap on_exit EXIT

attempt=0
[[ ! -f $attempt_file ]] || attempt=$(<"$attempt_file")
[[ $attempt =~ ^[0-9]+$ ]] || fail 'invalid durable attempt counter'
rm -f -- "$result_file"
write_file "$attempt_file" "$((attempt + 1))"

if systemctl cat k3s.service >/dev/null 2>&1; then
  service=k3s
elif systemctl cat k3s-agent.service >/dev/null 2>&1; then
  service=k3s-agent
else
  fail 'neither k3s nor k3s-agent is installed'
fi

wait_host_health() {
  local attempt
  for attempt in $(seq 1 90); do
    if systemctl is-active --quiet "$service" \
      && timeout 10 k3s ctr plugins ls 2>/dev/null \
        | awk 'tolower($0) ~ /cri/ && tolower($0) ~ /(^|[[:space:]])ok([[:space:]]|$)/ { found=1 } END { exit(found ? 0 : 1) }' \
      && { [[ $service == k3s-agent ]] || [[ $(timeout 10 k3s kubectl get --raw /readyz 2>/dev/null) == ok ]]; }; then
      return 0
    fi
    sleep 2
  done
  return 1
}

assert_default_runc() {
  local path=$1 assignment
  [[ -f $path && ! -L $path ]] || return 1
  assignment=$(awk -F= '/^[[:space:]]*default_runtime_name[[:space:]]*=/{gsub(/[[:space:]\"]/, "", $2); print $2}' "$path")
  [[ -z $assignment || $assignment == runc ]] || return 1
  ! grep -Fq 'runtime_type = "io.containerd.runsc.v1"' "$path" || return 1
}

verify_expected_installation() {
  [[ -x $version_dir/runsc && -x $version_dir/containerd-shim-runsc-v1 ]] || return 1
  [[ -L $runsc_link && $(readlink "$runsc_link") == "$version_dir/runsc" ]] || return 1
  [[ -L $shim_link && $(readlink "$shim_link") == "$version_dir/containerd-shim-runsc-v1" ]] || return 1
  grep -Fqx "binary_name = \"$version_dir/runsc\"" "$runsc_config" || return 1
  grep -Fqx '  overlay2 = "root:memory,size=128m"' "$runsc_config" || return 1
  grep -Fqx "  ConfigPath = \"$runsc_config\"" "$template" || return 1
  grep -Fqx "  ConfigPath = \"$runsc_config\"" "$rendered" || return 1
  grep -Fqx '  runtime_type = "io.containerd.runsc.v1"' "$rendered" || return 1
  assignment=$(awk -F= '/^[[:space:]]*default_runtime_name[[:space:]]*=/{gsub(/[[:space:]\"]/, "", $2); print $2}' "$rendered")
  [[ -z $assignment || $assignment == runc ]] || return 1
  timeout 10 "$version_dir/runsc" --version >/dev/null || return 1
}

restore_files() {
  local temporary
  if [[ -f $transaction_root/original-template && ! -L $transaction_root/original-template ]]; then
    [[ ! -L $template ]] || return 1
  elif [[ -f $transaction_root/original-template.absent && ! -L $transaction_root/original-template.absent ]]; then
    if [[ -e $template || -L $template ]]; then
      [[ -f $template && ! -L $template ]] || return 1
      grep -Fqx "  ConfigPath = \"$runsc_config\"" "$template" || return 1
    fi
  else
    return 1
  fi
  [[ ! -e $runsc_link && ! -L $runsc_link ]] \
    || [[ -L $runsc_link && $(readlink "$runsc_link") == "$version_dir/runsc" ]] || return 1
  [[ ! -e $shim_link && ! -L $shim_link ]] \
    || [[ -L $shim_link && $(readlink "$shim_link") == "$version_dir/containerd-shim-runsc-v1" ]] || return 1
  [[ ! -e $version_dir && ! -L $version_dir ]] \
    || [[ -d $version_dir && ! -L $version_dir && $version_dir == /usr/local/lib/gvisor/* ]] || return 1

  if [[ -f $transaction_root/original-template ]]; then
    temporary=$(mktemp "$(dirname "$template")/.mwc-gvisor-restore.XXXXXX")
    cp --preserve=mode,ownership,timestamps "$transaction_root/original-template" "$temporary"
    mv -T "$temporary" "$template"
  elif [[ -e $template ]]; then
    rm -f -- "$template"
  fi
  [[ ! -L $runsc_link ]] || rm -f -- "$runsc_link"
  [[ ! -L $shim_link ]] || rm -f -- "$shim_link"
  [[ ! -e $version_dir ]] || rm -rf -- "$version_dir"
}

restore_scheduling() {
  [[ -f $transaction_root/was-unschedulable ]] || return 1
  if [[ $(<"$transaction_root/was-unschedulable") == false ]]; then
    timeout 20 k3s kubectl uncordon "$expected_node" >/dev/null || return 1
  fi
}

recover_installation() {
  phase=RECOVERING
  state RECOVERING
  restore_files || return 1
  timeout 120 systemctl restart "$service" || return 1
  wait_host_health || return 1
  assert_default_runc "$rendered" || return 1
  restore_scheduling || return 1
}

if [[ $mode == recover ]]; then
  if [[ -f $result_file && $(<"$result_file") == ROLLED_BACK ]]; then
    finalized=true
    exit 0
  fi
  if recover_installation; then
    finish ROLLED_BACK
    exit 0
  fi
  finish RECOVERY_FAILED
  exit 1
fi

timeout 10 k3s kubectl get --raw /readyz >/dev/null || fail 'Kubernetes API is not ready before maintenance'
timeout 30 k3s kubectl wait --for=condition=Ready node \
  -l node-role.kubernetes.io/control-plane=true --timeout=20s >/dev/null \
  || fail 'not every control-plane node is Ready before maintenance'

if [[ ! -f $transaction_root/was-unschedulable ]]; then
  node_unschedulable=$(timeout 10 k3s kubectl get node "$expected_node" -o jsonpath='{.spec.unschedulable}')
  [[ $node_unschedulable == true ]] || node_unschedulable=false
  write_file "$transaction_root/was-unschedulable" "$node_unschedulable"
fi
timeout 20 k3s kubectl label node "$expected_node" \
  sandbox.memeloop.dev/tenant-ready- sandbox.memeloop.dev/gvisor-ready- --overwrite >/dev/null
timeout 20 k3s kubectl cordon "$expected_node" >/dev/null

if [[ $mode == rollback ]]; then
  phase=APPLYING
  state APPLYING
  if "$transaction_root/gvisor-node-rollback.sh" "$expected_node" "$transaction_id" "$checksum" "$expected_host" \
    && wait_host_health && restore_scheduling; then
    rm -f -- "$transaction_root/result-install"
    finish SUCCEEDED
    exit 0
  fi
  finish RECOVERY_FAILED
  exit 1
fi

if verify_expected_installation && wait_host_health; then
  restore_scheduling || { finish RECOVERY_FAILED; exit 1; }
  finish SUCCEEDED
  exit 0
fi

if [[ ! -e $transaction_root/original-template && ! -e $transaction_root/original-template.absent ]]; then
  if [[ -f $template && ! -L $template ]]; then
    cp --preserve=mode,ownership,timestamps "$template" "$transaction_root/original-template"
    chmod 0600 "$transaction_root/original-template"
  else
    : >"$transaction_root/original-template.absent"
    chmod 0600 "$transaction_root/original-template.absent"
  fi
fi

phase=APPLYING
state APPLYING
set +e
timeout 360 "$transaction_root/gvisor-node-install.sh" \
  --node "$expected_host" \
  --archive "$transaction_root/gvisor-x86_64.tar.bz2" \
  --sha256 "$checksum" \
  --rootfs-memory-mib 128 --apply --restart-k3s
install_status=$?
set -e

if (( install_status == 0 )) && wait_host_health && verify_expected_installation; then
  restore_scheduling || { finish RECOVERY_FAILED; exit 1; }
  finish SUCCEEDED
  exit 0
fi

if recover_installation; then
  finish ROLLED_BACK
  exit 1
fi
finish RECOVERY_FAILED
exit 1
