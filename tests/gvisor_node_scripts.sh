#!/usr/bin/env bash
# The fake command bodies below are single-quoted source text by design.
# shellcheck disable=SC2016
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
installer=$repo_root/scripts/gvisor-node-install.sh
preflight=$repo_root/scripts/gvisor-node-preflight.sh
fixture=$(mktemp -d)
trap 'rm -rf -- "$fixture"' EXIT

host_name=$(hostname)
fake_bin=$fixture/fake-bin
mkdir -p "$fake_bin"

write_fake_commands() {
  printf '%s\n' \
    '#!/usr/bin/env bash' \
    'set -euo pipefail' \
    'log=${GVISOR_FAKE_LOG:?}' \
    'if [[ ${1:-} == --version ]]; then printf "systemd 252\\n"; exit 0; fi' \
    'if [[ ${1:-} == is-active ]]; then' \
    '  unit=${3:-}' \
    '  [[ $unit == k3s && ${GVISOR_FAKE_K3S_ACTIVE:-true} == true ]] && exit 0' \
    '  [[ $unit == k3s-agent && ${GVISOR_FAKE_K3S_AGENT_ACTIVE:-false} == true ]] && exit 0' \
    '  exit 3' \
    'fi' \
    'if [[ ${1:-} == restart ]]; then' \
    '  printf "restart %s\\n" "${2:-}" >> "$log"' \
    '  if [[ ${GVISOR_FAKE_RESTART_FAIL:-false} == true ]]; then exit 1; fi' \
    '  cp -- "${GVISOR_FAKE_TEMPLATE:?}" "${GVISOR_FAKE_RENDERED:?}"' \
    '  exit 0' \
    'fi' \
    'printf "unexpected systemctl invocation: %q\\n" "$*" >&2' \
    'exit 2' > "$fake_bin/systemctl"
  chmod 0755 "$fake_bin/systemctl"

  printf '%s\n' \
    '#!/usr/bin/env bash' \
    'set -euo pipefail' \
    'log=${GVISOR_FAKE_LOG:?}' \
    'unit=${1:-}' \
    'action=${2:-}' \
    'if [[ $action == status ]]; then' \
    '  [[ $unit == k3s && ${GVISOR_FAKE_K3S_ACTIVE:-true} == true ]] && exit 0' \
    '  [[ $unit == k3s-agent && ${GVISOR_FAKE_K3S_AGENT_ACTIVE:-false} == true ]] && exit 0' \
    '  exit 3' \
    'fi' \
    'if [[ $action == restart ]]; then' \
    '  printf "restart %s\\n" "$unit" >> "$log"' \
    '  if [[ ${GVISOR_FAKE_RESTART_FAIL:-false} == true ]]; then exit 1; fi' \
    '  cp -- "${GVISOR_FAKE_TEMPLATE:?}" "${GVISOR_FAKE_RENDERED:?}"' \
    '  exit 0' \
    'fi' \
    'printf "unexpected rc-service invocation: %q\\n" "$*" >&2' \
    'exit 2' > "$fake_bin/rc-service"
  chmod 0755 "$fake_bin/rc-service"

  printf '%s\n' \
    '#!/usr/bin/env bash' \
    'set -euo pipefail' \
    'if [[ ${1:-} == ctr && ${2:-} == version ]]; then' \
    '  printf "Client:\\n  Version: v2.3.2-k3s2\\n"' \
    '  # Continue writing after the matching line so an early-exit awk gets SIGPIPE.' \
    '  head -c 8388608 /dev/zero' \
    '  exit 0' \
    'fi' \
    '[[ ${1:-} == ctr && ${2:-} == plugins && ${3:-} == ls ]] && { printf "io.containerd.cri.v1 runtime linux/amd64 ok\\n"; exit 0; }' \
    'printf "unexpected k3s invocation: %q\\n" "$*" >&2' \
    'exit 2' > "$fake_bin/k3s"
  chmod 0755 "$fake_bin/k3s"
}

make_fixture() {
  rm -rf -- "${fixture:?}/var" "${fixture:?}/usr" "${fixture:?}/sys" "${fixture:?}/incoming.tar" "${fixture:?}/package" "${fixture:?}/systemctl.log"
  mkdir -p \
    "$fixture/var/lib/rancher/k3s/agent/etc/containerd" \
    "$fixture/usr/local/bin" \
    "$fixture/usr/local/lib" \
    "$fixture/sys/fs/cgroup" \
    "$fixture/package/gvisor-bin"
  printf '%s\n' \
    '{{ template "base" . }}' \
    'default_runtime_name = "runc"' \
    > "$fixture/var/lib/rancher/k3s/agent/etc/containerd/config-v3.toml.tmpl"
  printf '%s\n' \
    'version = 3' \
    'default_runtime_name = "runc"' \
    > "$fixture/var/lib/rancher/k3s/agent/etc/containerd/config.toml"
  write_fake_commands
}

make_archive() {
  local extra=${1:-false}
  # A real tar archive exercises the newline member list and recursive directory entry.
  printf '%s\n' '#!/usr/bin/env bash' 'exit 0' > "$fixture/package/runsc"
  printf '%s\n' '#!/usr/bin/env bash' 'exit 0' > "$fixture/package/containerd-shim-runsc-v1"
  printf '%s\n' '#!/usr/bin/env bash' 'exit 0' > "$fixture/package/gvisor-bin/runsc-sandbox"
  chmod 0755 "$fixture/package/runsc" "$fixture/package/containerd-shim-runsc-v1" "$fixture/package/gvisor-bin/runsc-sandbox"
  if [[ $extra == true ]]; then
    printf 'unexpected\n' > "$fixture/package/unexpected"
    tar -C "$fixture/package" -cf "$fixture/incoming.tar" runsc containerd-shim-runsc-v1 gvisor-bin unexpected
  else
    tar -C "$fixture/package" -cf "$fixture/incoming.tar" runsc containerd-shim-runsc-v1 gvisor-bin
  fi
  checksum=$(sha256sum "$fixture/incoming.tar" | awk '{print $1}')
}

run_install() {
  PATH="$fake_bin:$PATH" \
  GVISOR_FAKE_LOG="$fixture/systemctl.log" \
  GVISOR_FAKE_TEMPLATE="$fixture/var/lib/rancher/k3s/agent/etc/containerd/config-v3.toml.tmpl" \
  GVISOR_FAKE_RENDERED="$fixture/var/lib/rancher/k3s/agent/etc/containerd/config.toml" \
  GVISOR_NODE_TEST_MODE=1 \
  GVISOR_NODE_TEST_ROOT="$fixture" \
  GVISOR_NODE_TEST_HOSTNAME="$host_name" \
  GVISOR_NODE_TEST_ARCH=x86_64 \
  GVISOR_NODE_TEST_KERNEL=5.15.0-test \
  GVISOR_NODE_TEST_CGROUP_FS=cgroup2fs \
  GVISOR_NODE_TEST_SYSTEMD_VERSION=252 \
  GVISOR_NODE_TEST_INIT="${GVISOR_NODE_TEST_INIT:-systemd}" \
  GVISOR_NODE_TEST_SERVICE="${GVISOR_NODE_TEST_SERVICE:-k3s}" \
  bash "$installer" --node "$host_name" --archive "$fixture/incoming.tar" --sha256 "$checksum" --apply --restart-k3s "$@"
}

run_preflight() {
  PATH="$fake_bin:$PATH" \
  GVISOR_FAKE_LOG="$fixture/systemctl.log" \
  GVISOR_NODE_TEST_MODE=1 \
  GVISOR_NODE_TEST_ROOT="$fixture" \
  GVISOR_NODE_TEST_ARCH=x86_64 \
  GVISOR_NODE_TEST_KERNEL=5.15.0-test \
  GVISOR_NODE_TEST_CGROUP_FS="${GVISOR_NODE_TEST_CGROUP_FS:-cgroup2fs}" \
  GVISOR_NODE_TEST_SYSTEMD_VERSION=252 \
  GVISOR_NODE_TEST_INIT="${GVISOR_NODE_TEST_INIT:-systemd}" \
  GVISOR_NODE_TEST_SERVICE="${GVISOR_NODE_TEST_SERVICE:-k3s}" \
  bash "$preflight"
}

make_fixture
make_archive
output=$(run_install 2>&1)
version_dir="$fixture/usr/local/lib/gvisor/$checksum"
template="$fixture/var/lib/rancher/k3s/agent/etc/containerd/config-v3.toml.tmpl"
rendered="$fixture/var/lib/rancher/k3s/agent/etc/containerd/config.toml"
[[ $output == *'PASS: REGISTERED runsc handler'* ]]
[[ $output == *'NOT ACCEPTANCE:'* ]]
[[ -x $version_dir/runsc && -x $version_dir/containerd-shim-runsc-v1 && -x $version_dir/gvisor-bin/runsc-sandbox ]]
[[ $(readlink "$fixture/usr/local/bin/runsc") == "$version_dir/runsc" ]]
[[ $(readlink "$fixture/usr/local/bin/containerd-shim-runsc-v1") == "$version_dir/containerd-shim-runsc-v1" ]]
grep -Fqx '  TypeUrl = "io.containerd.runsc.v1.options"' "$template"
grep -Fqx "  ConfigPath = \"/usr/local/lib/gvisor/$checksum/runsc.toml\"" "$template"
grep -Fqx '  TypeUrl = "io.containerd.runsc.v1.options"' "$rendered"
grep -Fqx 'default_runtime_name = "runc"' "$rendered"
grep -Fqx '  systemd-cgroup = "true"' "$version_dir/runsc.toml"
[[ $(grep -c '^restart k3s$' "$fixture/systemctl.log") == 1 ]]
backup_count=$(find "$fixture/var/lib/rancher/k3s/agent/etc/containerd/gvisor-backups" -type f -name 'config-v3.toml.tmpl.*' | wc -l)
[[ $backup_count == 1 ]]

make_fixture
make_archive
printf '%s\n' \
  '{{ template "base" . }}' \
  'default_runtime_name = "runc"' \
  '[plugins.'"'"'io.containerd.cri.v1.runtime'"'"'.containerd.runtimes.runsc]' \
  > "$template"
if run_install >/dev/null 2>&1; then
  printf 'existing runsc stanza was accepted\n' >&2
  exit 1
fi
[[ ! -e "$fixture/usr/local/lib/gvisor/$checksum" && ! -e "$fixture/usr/local/bin/runsc" ]]
[[ ! -e "$fixture/systemctl.log" ]]

make_fixture
make_archive true
if run_install >/dev/null 2>&1; then
  printf 'unexpected archive layout was accepted\n' >&2
  exit 1
fi
[[ ! -e "$fixture/usr/local/lib/gvisor/$checksum" ]]
[[ ! -e "$fixture/usr/local/bin/runsc" ]]

make_fixture
make_archive
original_template=$fixture/original-template
cp -- "$fixture/var/lib/rancher/k3s/agent/etc/containerd/config-v3.toml.tmpl" "$original_template"
if GVISOR_FAKE_RESTART_FAIL=true run_install >/dev/null 2>&1; then
  printf 'restart failure was accepted\n' >&2
  exit 1
fi
cmp -s "$original_template" "$fixture/var/lib/rancher/k3s/agent/etc/containerd/config-v3.toml.tmpl"
[[ ! -e "$fixture/usr/local/lib/gvisor/$checksum" ]]
[[ ! -e "$fixture/usr/local/bin/runsc" && ! -e "$fixture/usr/local/bin/containerd-shim-runsc-v1" ]]
[[ $(grep -c '^restart k3s$' "$fixture/systemctl.log") == 1 ]]

make_fixture
make_archive
if GVISOR_NODE_TEST_CGROUP_FS=tmpfs run_preflight >/dev/null 2>&1; then
  :
else
  printf 'cgroup-v1 preflight failed unexpectedly\n' >&2
  exit 1
fi

make_fixture
make_archive
printf '%s\n' 'default_runtime_name = "runsc"' > "$fixture/var/lib/rancher/k3s/agent/etc/containerd/config.toml"
if run_install >/dev/null 2>&1; then
  printf 'non-runc default was accepted\n' >&2
  exit 1
fi
[[ ! -e "$fixture/usr/local/lib/gvisor/$checksum" ]]

make_fixture
make_archive
printf 'old runsc\n' > "$fixture/usr/local/bin/runsc"
if run_install >/dev/null 2>&1; then
  printf 'existing runsc path was replaced\n' >&2
  exit 1
fi
grep -Fqx 'old runsc' "$fixture/usr/local/bin/runsc"
[[ ! -e "$fixture/usr/local/lib/gvisor/$checksum" ]]

if PATH="$fake_bin:$PATH" GVISOR_NODE_TEST_MODE=1 GVISOR_NODE_TEST_ROOT="$fixture" \
  bash "$installer" --node not-this-host --archive "$fixture/incoming.tar" --sha256 "$checksum" --apply --restart-k3s >/dev/null 2>&1; then
  printf 'hostname mismatch was accepted\n' >&2
  exit 1
fi

make_fixture
make_archive
run_install --rootfs-memory-mib 128 >/dev/null
grep -Fqx '  overlay2 = "root:memory,size=128m"' "$fixture/usr/local/lib/gvisor/$checksum/runsc.toml"

make_fixture
make_archive
GVISOR_NODE_TEST_INIT=openrc \
GVISOR_NODE_TEST_SERVICE=k3s-agent \
GVISOR_FAKE_K3S_ACTIVE=false \
GVISOR_FAKE_K3S_AGENT_ACTIVE=true \
  run_install --rootfs-memory-mib 128 >/dev/null
if grep -Fq 'systemd-cgroup' "$fixture/usr/local/lib/gvisor/$checksum/runsc.toml"; then
  printf 'OpenRC installation enabled systemd cgroups\n' >&2
  exit 1
fi
grep -Fqx '  overlay2 = "root:memory,size=128m"' "$fixture/usr/local/lib/gvisor/$checksum/runsc.toml"
grep -Fqx 'restart k3s-agent' "$fixture/systemctl.log"

make_fixture
make_archive
if run_install --rootfs-memory-mib 0 >/dev/null 2>&1; then
  printf 'invalid root filesystem memory limit was accepted\n' >&2
  exit 1
fi
[[ ! -e "$fixture/usr/local/lib/gvisor/$checksum" ]]

printf 'gVisor node installer/preflight fixture tests passed\n'
