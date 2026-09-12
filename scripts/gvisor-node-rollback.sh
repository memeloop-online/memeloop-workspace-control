#!/usr/bin/env bash
# Removes one known GitOps-installed runsc handler after its scheduling gates are closed.
set -euo pipefail

fail() { printf 'FAIL: %s\n' "$*" >&2; exit 1; }

expected_node=${1:-}
transaction_id=${2:-}
checksum=${3:-}
expected_host=${4:-$expected_node}
transaction_root="/var/lib/memeloop-workspace-control/node-maintenance/$transaction_id"
template=/var/lib/rancher/k3s/agent/etc/containerd/config-v3.toml.tmpl
version_dir="/usr/local/lib/gvisor/$checksum"
runsc_link=/usr/local/bin/runsc
shim_link=/usr/local/bin/containerd-shim-runsc-v1

[[ $expected_node =~ ^[a-z0-9][a-z0-9.-]{0,62}$ ]] || fail 'invalid node name'
[[ $expected_host =~ ^[a-z0-9][a-z0-9.-]{0,62}$ ]] || fail 'invalid host name'
[[ $transaction_id =~ ^[a-z0-9][a-z0-9-]{0,62}$ ]] || fail 'invalid transaction ID'
[[ $checksum =~ ^[a-f0-9]{64}$ ]] || fail 'invalid checksum'
IFS= read -r host_name </proc/sys/kernel/hostname
[[ $host_name == "$expected_host" ]] || fail 'host name does not match the approved node mapping'
[[ -d $transaction_root && ! -L $transaction_root ]] || fail 'transaction directory is unsafe'

runtime_classes=$(k3s kubectl get runtimeclass -o go-template='{{range .items}}{{if eq .handler "runsc"}}{{.metadata.name}}{{"\n"}}{{end}}{{end}}')
while IFS= read -r runtime_class; do
  [[ -n $runtime_class ]] || continue
  running=$(k3s kubectl get pods -A --field-selector "spec.nodeName=$expected_node" \
    -o jsonpath="{range .items[?(@.spec.runtimeClassName==\"$runtime_class\")]}{.metadata.namespace}/{.metadata.name}{\"\\n\"}{end}")
  [[ -z $running ]] || fail "runsc Pods still run on the node: $running"
done <<<"$runtime_classes"
while IFS= read -r sandbox_id; do
  [[ -n $sandbox_id ]] || continue
  if k3s crictl inspectp "$sandbox_id" 2>/dev/null | grep -Eq '"runtimeHandler"[[:space:]]*:[[:space:]]*"runsc"'; then
    fail "a runsc sandbox still exists on the node: $sandbox_id"
  fi
done < <(k3s crictl pods -q 2>/dev/null)

# Validate every owned target before changing any of them.
if [[ -f $transaction_root/original-template && ! -L $transaction_root/original-template ]]; then
  [[ ! -L $template ]] || fail 'containerd template target is a symlink'
elif [[ -f $transaction_root/original-template.absent && ! -L $transaction_root/original-template.absent ]]; then
  if [[ -e $template || -L $template ]]; then
    [[ -f $template && ! -L $template ]] || fail 'generated template is unsafe'
    grep -Fqx "  ConfigPath = \"$version_dir/runsc.toml\"" "$template" \
      || fail 'generated template does not belong to this transaction'
  fi
else
  fail 'original template state is unavailable'
fi
[[ ! -e $runsc_link && ! -L $runsc_link ]] \
  || [[ -L $runsc_link && $(readlink "$runsc_link") == "$version_dir/runsc" ]] \
  || fail 'runsc path belongs to another installation'
[[ ! -e $shim_link && ! -L $shim_link ]] \
  || [[ -L $shim_link && $(readlink "$shim_link") == "$version_dir/containerd-shim-runsc-v1" ]] \
  || fail 'containerd shim path belongs to another installation'
[[ ! -e $version_dir && ! -L $version_dir ]] \
  || [[ -d $version_dir && ! -L $version_dir && $version_dir == /usr/local/lib/gvisor/* ]] \
  || fail 'version directory is unsafe'

if [[ -f $transaction_root/original-template && ! -L $transaction_root/original-template ]]; then
  temporary=$(mktemp "$(dirname "$template")/.mwc-gvisor-restore.XXXXXX")
  cp --preserve=mode,ownership,timestamps "$transaction_root/original-template" "$temporary"
  mv -T "$temporary" "$template"
elif [[ -f $transaction_root/original-template.absent ]]; then
  [[ ! -e $template ]] || rm -f -- "$template"
fi

if [[ -L $runsc_link ]]; then
  [[ $(readlink "$runsc_link") == "$version_dir/runsc" ]] \
    || fail 'runsc link belongs to another installation'
  rm -f -- "$runsc_link"
fi
if [[ -L $shim_link ]]; then
  [[ $(readlink "$shim_link") == "$version_dir/containerd-shim-runsc-v1" ]] \
    || fail 'containerd shim link belongs to another installation'
  rm -f -- "$shim_link"
fi
if [[ -e $version_dir || -L $version_dir ]]; then
  [[ -d $version_dir && ! -L $version_dir && $version_dir == /usr/local/lib/gvisor/* ]] \
    || fail 'version directory is unsafe'
  rm -rf -- "$version_dir"
fi

if systemctl cat k3s.service >/dev/null 2>&1; then
  service=k3s
elif systemctl cat k3s-agent.service >/dev/null 2>&1; then
  service=k3s-agent
else
  fail 'neither k3s nor k3s-agent is installed'
fi
systemctl restart "$service"
for _ in $(seq 1 90); do
  if systemctl is-active --quiet "$service" \
    && k3s ctr plugins ls 2>/dev/null \
      | awk 'tolower($0) ~ /cri/ && tolower($0) ~ /(^|[[:space:]])ok([[:space:]]|$)/ { found=1 } END { exit(found ? 0 : 1) }'; then
    exit 0
  fi
  sleep 2
done
fail 'K3s did not recover after rollback'
