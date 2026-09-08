#!/usr/bin/env bash
# Read-only suitability check for one K3s node before enabling the runsc handler.
set -euo pipefail

fail() { printf 'FAIL: %s\n' "$*" >&2; exit 1; }
warn() { printf 'WARN: %s\n' "$*" >&2; }

[[ $(id -u) -eq 0 ]] || fail 'run as root so K3s paths can be inspected'
command -v k3s >/dev/null 2>&1 || fail 'k3s is not on PATH'
command -v runsc >/dev/null 2>&1 || warn 'runsc is not installed yet (expected before installation)'

arch=$(uname -m)
case "$arch" in
  x86_64|aarch64) ;;
  *) fail "unsupported architecture: $arch" ;;
esac

kernel=$(uname -r)
kernel_major=${kernel%%.*}
kernel_rest=${kernel#*.}
kernel_minor=${kernel_rest%%.*}
[[ $kernel_major =~ ^[0-9]+$ && $kernel_minor =~ ^[0-9]+$ ]] || fail "cannot parse kernel version: $kernel"
if (( kernel_major < 5 || (kernel_major == 5 && kernel_minor < 6) )); then
  fail "kernel $kernel is below gVisor's Linux 5.6 minimum"
fi

if systemctl is-active --quiet k3s 2>/dev/null; then
  service=k3s
elif systemctl is-active --quiet k3s-agent 2>/dev/null; then
  service=k3s-agent
else
  fail 'neither k3s nor k3s-agent is active'
fi

containerd_version=$(k3s ctr version 2>/dev/null | awk '/Version:/ { print $2; exit }')
[[ -n $containerd_version ]] || fail 'could not determine embedded containerd version'
case "$containerd_version" in
  2.*) template=/var/lib/rancher/k3s/agent/etc/containerd/config-v3.toml.tmpl ;;
  *) fail "containerd $containerd_version is not covered by this K3s containerd-v3 procedure" ;;
esac

mountpoint -q /sys/fs/cgroup || fail 'cgroup filesystem is not mounted at /sys/fs/cgroup'
cgroup_fs=$(stat -fc %T /sys/fs/cgroup)
case "$cgroup_fs" in
  cgroup2fs) cgroup_mode=v2 ;;
  tmpfs) cgroup_mode=v1-or-hybrid ;;
  *) warn "unrecognized cgroup filesystem type: $cgroup_fs"; cgroup_mode=unknown ;;
esac

if command -v getenforce >/dev/null 2>&1 && [[ $(getenforce) == Enforcing ]]; then
  warn 'SELinux is Enforcing; validate gVisor compatibility in a canary before production use'
fi
if [[ -e /proc/sys/kernel/unprivileged_userns_clone ]] && [[ $(cat /proc/sys/kernel/unprivileged_userns_clone) != 1 ]]; then
  warn 'unprivileged user namespaces are disabled; record this result in canary evidence'
fi

printf 'PASS: service=%s arch=%s containerd=%s template=%s cgroup=%s kernel=%s\n' \
  "$service" "$arch" "$containerd_version" "$template" "$cgroup_mode" "$kernel"
printf 'NEXT: inspect node labels/taints and capacity from the API; do not label a node based on this check alone.\n'
