#!/usr/bin/env bash
# Read-only suitability check for one K3s node before enabling the runsc handler.
#
# GVISOR_NODE_TEST_MODE and GVISOR_NODE_TEST_ROOT are intentionally test-only
# escape hatches. They make the checks runnable against a disposable fixture;
# they never select host paths unless both variables are set by a test.
set -euo pipefail

fail() { printf 'FAIL: %s\n' "$*" >&2; exit 1; }
warn() { printf 'WARN: %s\n' "$*" >&2; }

test_mode=${GVISOR_NODE_TEST_MODE:-0}
fixture_root=
if [[ $test_mode == 1 ]]; then
  fixture_root=${GVISOR_NODE_TEST_ROOT:-}
  [[ -n $fixture_root && $fixture_root == /* && $fixture_root != / ]] \
    || fail 'test mode requires an absolute, non-root GVISOR_NODE_TEST_ROOT'
  [[ $fixture_root != *$'\n'* && $fixture_root != *'..'* ]] \
    || fail 'test fixture root must not contain newlines or ..'
  fixture_root=${fixture_root%/}
else
  [[ $(id -u) -eq 0 ]] || fail 'run as root so K3s paths can be inspected'
fi

map_path() {
  if [[ -n $fixture_root ]]; then
    printf '%s%s' "$fixture_root" "$1"
  else
    printf '%s' "$1"
  fi
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || fail "$1 is not on PATH"
}

for command_name in awk command dirname mountpoint stat systemctl uname; do
  require_command "$command_name"
done
require_command k3s

if command -v runsc >/dev/null 2>&1; then
  runsc_state=installed
else
  warn 'runsc is not installed yet (expected before installation)'
  runsc_state=absent
fi

if [[ $test_mode == 1 && -n ${GVISOR_NODE_TEST_ARCH:-} ]]; then
  arch=$GVISOR_NODE_TEST_ARCH
else
  arch=$(uname -m)
fi
case "$arch" in
  x86_64|aarch64) ;;
  *) fail "unsupported architecture: $arch" ;;
esac

if [[ $test_mode == 1 && -n ${GVISOR_NODE_TEST_KERNEL:-} ]]; then
  kernel=$GVISOR_NODE_TEST_KERNEL
else
  kernel=$(uname -r)
fi
kernel_major=${kernel%%.*}
kernel_rest=${kernel#*.}
kernel_minor=${kernel_rest%%.*}
[[ $kernel_major =~ ^[0-9]+$ && $kernel_minor =~ ^[0-9]+$ ]] \
  || fail "cannot parse kernel version: $kernel"
if (( kernel_major < 5 || (kernel_major == 5 && kernel_minor < 6) )); then
  fail "kernel $kernel is below gVisor's Linux 5.6 minimum"
fi

service=
if [[ $test_mode == 1 && -n ${GVISOR_NODE_TEST_SERVICE:-} ]]; then
  service=$GVISOR_NODE_TEST_SERVICE
  [[ $service == k3s || $service == k3s-agent ]] || fail "invalid test service: $service"
else
  if systemctl is-active --quiet k3s 2>/dev/null; then
    service=k3s
  elif systemctl is-active --quiet k3s-agent 2>/dev/null; then
    service=k3s-agent
  else
    fail 'neither k3s nor k3s-agent is active'
  fi
fi

if [[ $test_mode == 1 && -n ${GVISOR_NODE_TEST_CONTAINERD_VERSION:-} ]]; then
  containerd_version=${GVISOR_NODE_TEST_CONTAINERD_VERSION#v}
else
  containerd_version=$(k3s ctr version 2>/dev/null \
    | awk '/^[[:space:]]*Version:[[:space:]]*/ { value=$2; sub(/^v/, "", value); print value; exit }')
fi
[[ -n $containerd_version ]] || fail 'could not determine embedded containerd version'
containerd_major=${containerd_version%%.*}
[[ $containerd_major =~ ^[0-9]+$ ]] || fail "cannot parse containerd version: $containerd_version"
case "$containerd_major" in
  2) template_logical=/var/lib/rancher/k3s/agent/etc/containerd/config-v3.toml.tmpl ;;
  *) fail "containerd $containerd_version is not covered by this containerd-v3 procedure" ;;
esac

template=$(map_path "$template_logical")
rendered_logical=/var/lib/rancher/k3s/agent/etc/containerd/config.toml
rendered=$(map_path "$rendered_logical")
containerd_dir=$(dirname "$template")
[[ -d $containerd_dir && ! -L $containerd_dir ]] \
  || fail "K3s containerd directory is missing or unsafe: $containerd_dir"
[[ -f $rendered && ! -L $rendered ]] \
  || fail "rendered containerd config is missing or unsafe: $rendered"

assert_default_runc() {
  local path=$1 label=$2 line value found=false
  while IFS= read -r line; do
    if [[ $line =~ ^[[:space:]]*default_runtime_name[[:space:]]*=[[:space:]]*\"([^\"]+)\"[[:space:]]*(#.*)?$ ]]; then
      value=${BASH_REMATCH[1]}
      [[ $value == runc ]] || fail "$label sets default_runtime_name to $value, not runc"
      found=true
    else
      fail "$label has an invalid default_runtime_name assignment: $line"
    fi
  done < <(awk '/^[[:space:]]*default_runtime_name[[:space:]]*=/' "$path")
  if [[ $found == false ]]; then
    printf '%s: default runtime is implicitly runc\n' "$label"
  else
    printf '%s: default runtime is explicitly runc\n' "$label"
  fi
}

has_runsc_handler() {
  awk '
    /^[[:space:]]*#/ { next }
    /^[[:space:]]*\[/ && /containerd[.]runtimes/ && /runsc/ { found=1 }
    /^[[:space:]]*runtime_type[[:space:]]*=/ && /io[.]containerd[.]runsc[.]v1/ { found=1 }
    END { exit(found ? 0 : 1) }
  ' "$1"
}

if [[ -e $template || -L $template ]]; then
  [[ -f $template && ! -L $template ]] || fail "containerd template is not a regular file: $template"
  if has_runsc_handler "$template"; then
    warn "containerd template already contains a runsc handler: $template"
  fi
  assert_default_runc "$template" template
else
  warn "containerd template is absent; the installer will create a base template: $template"
fi
if has_runsc_handler "$rendered"; then
  warn "rendered containerd config already contains a runsc handler: $rendered"
fi
assert_default_runc "$rendered" rendered

cgroup_root=$(map_path /sys/fs/cgroup)
if [[ $test_mode == 1 && -n ${GVISOR_NODE_TEST_CGROUP_FS:-} ]]; then
  cgroup_fs=$GVISOR_NODE_TEST_CGROUP_FS
else
  require_command cat
  mountpoint -q "$cgroup_root" || fail "cgroup filesystem is not mounted at $cgroup_root"
  cgroup_fs=$(stat -fc %T "$cgroup_root")
fi
case "$cgroup_fs" in
  cgroup2fs)
    cgroup_mode=v2
    ;;
  tmpfs)
    cgroup_mode=v1-or-hybrid
    ;;
  *)
    warn "unrecognized cgroup filesystem type: $cgroup_fs"
    cgroup_mode=unknown
    ;;
esac

systemd_version=
if [[ $test_mode == 1 && -n ${GVISOR_NODE_TEST_SYSTEMD_VERSION:-} ]]; then
  systemd_version=$GVISOR_NODE_TEST_SYSTEMD_VERSION
elif [[ $cgroup_mode == v2 ]]; then
  systemd_version=$(systemctl --version 2>/dev/null \
    | awk 'NR == 1 { print $2; exit }')
fi
if [[ $cgroup_mode == v2 ]]; then
  [[ $systemd_version =~ ^[0-9]+$ ]] \
    || fail 'could not determine systemd version required for gVisor systemd cgroups'
  (( systemd_version >= 244 )) \
    || fail "systemd $systemd_version is below gVisor's 244 minimum for --systemd-cgroup"
  runsc_cgroup=systemd-v2
else
  runsc_cgroup=runsc-default-fs
fi

if command -v getenforce >/dev/null 2>&1; then
  selinux_state=$(getenforce 2>/dev/null || true)
  if [[ $selinux_state == Enforcing ]]; then
    warn 'SELinux is Enforcing; validate gVisor compatibility in a canary before production use'
  fi
fi
if [[ $test_mode == 1 && -n ${GVISOR_NODE_TEST_USERNS_CLONE:-} ]]; then
  userns_clone=$GVISOR_NODE_TEST_USERNS_CLONE
elif [[ -e /proc/sys/kernel/unprivileged_userns_clone ]]; then
  userns_clone=$(cat /proc/sys/kernel/unprivileged_userns_clone)
else
  userns_clone=
fi
if [[ -n $userns_clone && $userns_clone != 1 ]]; then
  warn 'unprivileged user namespaces are disabled; record this result in canary evidence'
fi

printf 'PASS: service=%s arch=%s containerd=%s template=%s rendered=%s cgroup=%s runsc-cgroup=%s kernel=%s runsc=%s\n' \
  "$service" "$arch" "$containerd_version" "$template_logical" "$rendered_logical" \
  "$cgroup_mode" "$runsc_cgroup" "$kernel" "$runsc_state"
printf 'NEXT: independently verify the host-to-Kubernetes-node mapping, labels/taints and capacity; this read-only check does not call the Kubernetes API or start a workload.\n'
