#!/usr/bin/env bash
# Cold worker boot, before K3s starts. Never mount over running tasks.
set -euo pipefail
bundle=/run/k3s/containerd/io.containerd.runtime.v2.task/k8s.io
logs=/var/log/pods
fail() { printf 'MWC scratch mounts: %s\n' "$*" >&2; exit 1; }
[[ $(readlink /proc/self/ns/mnt) == "$(readlink /proc/1/ns/mnt)" ]] || fail 'not in the node mount namespace'
if pgrep -x containerd >/dev/null; then fail 'containerd already running'; fi
if [[ ! -e $bundle ]]; then install -d -m 0711 "$bundle"; fi
if [[ ! -e $logs ]]; then install -d -m 0750 "$logs"; fi
for leaf in "$bundle" "$logs"; do
  [[ -d $leaf && $(realpath -e "$leaf") == "$leaf" ]] || fail "not a canonical directory: $leaf"
  read -r owner mode < <(stat -c '%u %a' "$leaf")
  [[ $owner == 0 ]] && (( (8#$mode & 0022) == 0 )) || fail "not a root-protected directory: $leaf"
  if awk -v p="$leaf" '$5==p || index($5,p "/")==1 {found=1} END {exit !found}' /proc/self/mountinfo; then
    fail "existing mounts at or below $leaf"
  fi
done
[[ -z $(find "$bundle" -mindepth 1 -print -quit) ]] || fail 'the runtime bundle is not empty'
logs_inode=$(stat -c '%d:%i' "$logs")
mount --bind "$bundle" "$bundle"
mount --make-private "$bundle"
mount --bind "$logs" "$logs"
[[ $(findmnt -n -M "$bundle" -o PROPAGATION) == private ]] || fail 'runtime bundle did not become private'
[[ $(stat -c '%d:%i' "$logs") == "$logs_inode" ]] || fail 'canonical log directory changed identity'
