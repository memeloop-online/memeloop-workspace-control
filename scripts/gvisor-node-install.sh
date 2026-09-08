#!/usr/bin/env bash
# Installs an explicitly supplied gVisor archive and registers a non-default K3s runtime handler.
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: gvisor-node-install.sh --node NAME --archive FILE --sha256 HEX --apply --restart-k3s

This script must run on the approved node as root. It never changes K3s' default runtime and it
does not add Kubernetes labels. --restart-k3s is explicit because K3s must regenerate/reload its
containerd configuration. Do not use it before a maintenance decision and a verified SSH/console
recovery path exist.
EOF
}
fail() { printf 'FAIL: %s\n' "$*" >&2; exit 1; }

node= archive= checksum= apply=false restart=false
while [[ $# -gt 0 ]]; do
  case "$1" in
    --node) node=${2:-}; shift 2 ;;
    --archive) archive=${2:-}; shift 2 ;;
    --sha256) checksum=${2:-}; shift 2 ;;
    --apply) apply=true; shift ;;
    --restart-k3s) restart=true; shift ;;
    -h|--help) usage; exit 0 ;;
    *) usage >&2; fail "unknown argument: $1" ;;
  esac
done
[[ $(id -u) -eq 0 ]] || fail 'run as root'
[[ -n $node && -n $archive && -n $checksum ]] || { usage >&2; fail 'node, archive, and sha256 are required'; }
[[ $apply == true && $restart == true ]] || fail 'refusing to alter a node without both --apply and --restart-k3s'
[[ $(hostname) == "$node" ]] || fail "--node ($node) does not match this host ($(hostname))"
[[ -r $archive ]] || fail "archive is unreadable: $archive"
[[ $checksum =~ ^[a-fA-F0-9]{64}$ ]] || fail 'sha256 must be 64 hexadecimal characters'

actual=$(sha256sum "$archive" | awk '{print $1}')
[[ ${actual,,} == ${checksum,,} ]] || fail 'archive checksum does not match --sha256'

"$(dirname "$0")/gvisor-node-preflight.sh"

workdir=$(mktemp -d)
trap 'rm -rf "$workdir"' EXIT
tar --extract --file "$archive" --directory "$workdir"
runsc_path=$(find "$workdir" -type f -name runsc -perm -u+x -print -quit)
shim_path=$(find "$workdir" -type f -name containerd-shim-runsc-v1 -perm -u+x -print -quit)
[[ -n $runsc_path && -n $shim_path ]] || fail 'archive must contain executable runsc and containerd-shim-runsc-v1'
bin_dir=/usr/local/lib/gvisor
install -d -m 0755 "$bin_dir"
cp -a "$(dirname "$runsc_path")/." "$bin_dir/"
install -m 0755 "$shim_path" "$bin_dir/containerd-shim-runsc-v1"
[[ -d "$bin_dir/gvisor-bin" ]] || fail 'archive did not place gvisor-bin beside runsc; refusing an incomplete install'
ln -sfn "$bin_dir/runsc" /usr/local/bin/runsc
ln -sfn "$bin_dir/containerd-shim-runsc-v1" /usr/local/bin/containerd-shim-runsc-v1

template=/var/lib/rancher/k3s/agent/etc/containerd/config-v3.toml.tmpl
backup_dir=/var/lib/rancher/k3s/agent/etc/containerd/gvisor-backups
install -d -m 0700 "$backup_dir"
if [[ -e $template ]]; then
  cp -a "$template" "$backup_dir/config-v3.toml.tmpl.$(date -u +%Y%m%dT%H%M%SZ)"
  grep -Fq "runtimes.runsc" "$template" && fail 'a runsc runtime stanza already exists; inspect it instead of overwriting it'
else
  cat >"$template" <<'EOF'
{{ template "base" . }}
EOF
fi
cat >>"$template" <<'EOF'

[plugins.'io.containerd.cri.v1.runtime'.containerd.runtimes.runsc]
  runtime_type = "io.containerd.runsc.v1"
[plugins.'io.containerd.cri.v1.runtime'.containerd.runtimes.runsc.options]
  TypeUrl = "io.containerd.runsc.v1.options"
EOF

if systemctl is-active --quiet k3s; then service=k3s; else service=k3s-agent; fi
printf 'Configured handler runsc in %s. Restarting %s only because it was explicitly requested.\n' "$template" "$service"
systemctl restart "$service"
systemctl is-active --quiet "$service" || fail "$service did not return active after restart"
grep -Fq "runtimes.runsc" /var/lib/rancher/k3s/agent/etc/containerd/config.toml || fail 'K3s did not render the runsc handler'
printf 'PASS: runsc handler rendered. Apply a RuntimeClass only after canary evidence succeeds.\n'
