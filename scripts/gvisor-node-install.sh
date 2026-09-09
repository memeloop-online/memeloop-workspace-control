#!/usr/bin/env bash
# Installs an explicitly supplied gVisor archive and registers a non-default K3s runtime handler.
#
# GVISOR_NODE_TEST_MODE and GVISOR_NODE_TEST_ROOT are intentionally test-only
# escape hatches. They make the checks runnable against a disposable fixture;
# they never select host paths unless both variables are set by a test.
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: gvisor-node-install.sh --node NAME --archive FILE --sha256 HEX --apply --restart-k3s [--rootfs-memory-mib N]

This script must run on the approved node as root. --node is compared with this host's hostname;
it is not a Kubernetes API lookup. If a Kubernetes Node name differs from the host name, verify
that mapping independently before running this script. The script never changes K3s' default
runtime or adds Kubernetes labels. --restart-k3s is explicit because K3s must regenerate/reload
its containerd configuration. Do not use it before a maintenance decision and a verified
SSH/console recovery path exist.
--rootfs-memory-mib bounds root-filesystem changes in application memory (16..4096 MiB).
Mounted volumes are unaffected. Omit this option to use runsc's default filesystem overlay.
EOF
}
fail() { printf 'FAIL: %s\n' "$*" >&2; exit 1; }
warn() { printf 'WARN: %s\n' "$*" >&2; }

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
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
  [[ $(id -u) -eq 0 ]] || fail 'run as root'
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

for command_name in awk cat chmod cp date dirname find grep hostname install ln mktemp mv readlink rm sha256sum stat tar; do
  require_command "$command_name"
done
require_command systemctl
require_command k3s

node='' archive='' checksum='' apply=false restart=false rootfs_memory_mib=''
while [[ $# -gt 0 ]]; do
  case "$1" in
    --node)
      [[ $# -ge 2 && -n ${2:-} ]] || fail '--node requires a value'
      node=$2
      shift 2
      ;;
    --archive)
      [[ $# -ge 2 && -n ${2:-} ]] || fail '--archive requires a value'
      archive=$2
      shift 2
      ;;
    --sha256)
      [[ $# -ge 2 && -n ${2:-} ]] || fail '--sha256 requires a value'
      checksum=$2
      shift 2
      ;;
    --apply)
      apply=true
      shift
      ;;
    --rootfs-memory-mib)
      [[ $# -ge 2 && ${2:-} =~ ^[1-9][0-9]{0,3}$ ]] \
        || fail '--rootfs-memory-mib requires an integer from 16 to 4096'
      rootfs_memory_mib=$2
      (( rootfs_memory_mib >= 16 && rootfs_memory_mib <= 4096 )) \
        || fail '--rootfs-memory-mib must be from 16 to 4096'
      shift 2
      ;;
    --restart-k3s)
      restart=true
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      usage >&2
      fail "unknown argument: $1"
      ;;
  esac
done

[[ -n $node && -n $archive && -n $checksum ]] \
  || { usage >&2; fail 'node, archive, and sha256 are required'; }
[[ $node != */* && $node != *$'\n'* ]] || fail '--node must be a hostname-like value, not a path'
[[ $apply == true && $restart == true ]] \
  || fail 'refusing to alter a node without both --apply and --restart-k3s'

if [[ $test_mode == 1 && -n ${GVISOR_NODE_TEST_HOSTNAME:-} ]]; then
  local_hostname=$GVISOR_NODE_TEST_HOSTNAME
else
  local_hostname=$(hostname)
fi
[[ -n $local_hostname ]] || fail 'could not determine this host name'
[[ $node == "$local_hostname" ]] \
  || fail "--node ($node) does not match this host ($local_hostname); --node is not a Kubernetes API lookup"

[[ -f $archive && ! -L $archive && -r $archive ]] \
  || fail "archive is missing, symlinked, or unreadable: $archive"
[[ $checksum =~ ^[a-fA-F0-9]{64}$ ]] || fail 'sha256 must be 64 hexadecimal characters'
actual=$(sha256sum -- "$archive" | awk '{print $1}')
[[ ${actual,,} == "${checksum,,}" ]] || fail 'archive checksum does not match --sha256'
checksum=${checksum,,}

preflight_script=$script_dir/gvisor-node-preflight.sh
[[ -x $preflight_script ]] || fail "preflight script is not executable: $preflight_script"
"$preflight_script"

template_logical=/var/lib/rancher/k3s/agent/etc/containerd/config-v3.toml.tmpl
rendered_logical=/var/lib/rancher/k3s/agent/etc/containerd/config.toml
backup_dir_logical=/var/lib/rancher/k3s/agent/etc/containerd/gvisor-backups
bin_root_logical=/usr/local/lib/gvisor
runsc_link_logical=/usr/local/bin/runsc
shim_link_logical=/usr/local/bin/containerd-shim-runsc-v1

template=$(map_path "$template_logical")
rendered=$(map_path "$rendered_logical")
backup_dir=$(map_path "$backup_dir_logical")
bin_root=$(map_path "$bin_root_logical")
runsc_link=$(map_path "$runsc_link_logical")
shim_link=$(map_path "$shim_link_logical")
containerd_dir=$(dirname "$template")
bin_parent=$(dirname "$bin_root")
links_parent=$(dirname "$runsc_link")
version_dir_logical=$bin_root_logical/$checksum
version_dir=$(map_path "$version_dir_logical")
runsc_config_logical=$version_dir_logical/runsc.toml

secure_mode() {
  local path=$1 label=$2 mode owner
  mode=$(stat -c %a -- "$path") || fail "could not inspect permissions for $label: $path"
  [[ $mode =~ ^[0-7]+$ ]] || fail "could not parse permissions for $label: $path"
  (( (8#$mode & 0022) == 0 )) || fail "$label is writable by group or other: $path"
  if [[ $test_mode != 1 ]]; then
    owner=$(stat -c %u -- "$path") || fail "could not inspect owner for $label: $path"
    [[ $owner == 0 ]] || fail "$label is not owned by root: $path"
  fi
}

secure_dir() {
  local path=$1 label=$2
  [[ -d $path && ! -L $path ]] || fail "$label is missing or unsafe: $path"
  secure_mode "$path" "$label"
}

secure_file() {
  local path=$1 label=$2
  [[ -f $path && ! -L $path ]] || fail "$label is missing or unsafe: $path"
  secure_mode "$path" "$label"
}

has_runsc_handler() {
  awk '
    /^[[:space:]]*#/ { next }
    /^[[:space:]]*\[/ && /containerd[.]runtimes/ && /runsc/ { found=1 }
    /^[[:space:]]*runtime_type[[:space:]]*=/ && /io[.]containerd[.]runsc[.]v1/ { found=1 }
    END { exit(found ? 0 : 1) }
  ' "$1"
}

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
  if [[ $found == true ]]; then
    printf '%s: default runtime explicitly runc\n' "$label"
  else
    printf '%s: default runtime implicitly runc\n' "$label"
  fi
}

assert_handler_file() {
  local path=$1 label=$2 config_path=$3 stanza option_stanza
  stanza="[plugins.'io.containerd.cri.v1.runtime'.containerd.runtimes.runsc]"
  option_stanza="[plugins.'io.containerd.cri.v1.runtime'.containerd.runtimes.runsc.options]"
  local stanza_count option_count
  stanza_count=$(awk -v expected="$stanza" '$0 == expected { count++ } END { print count + 0 }' "$path")
  option_count=$(awk -v expected="$option_stanza" '$0 == expected { count++ } END { print count + 0 }' "$path")
  [[ $stanza_count == 1 && $option_count == 1 ]] \
    || fail "$label does not contain exactly one runsc runtime stanza"
  grep -Fqx '  runtime_type = "io.containerd.runsc.v1"' "$path" \
    || fail "$label has an invalid runsc runtime_type"
  grep -Fqx '  TypeUrl = "io.containerd.runsc.v1.options"' "$path" \
    || fail "$label has an invalid runsc TypeUrl"
  grep -Fqx "  ConfigPath = \"$config_path\"" "$path" \
    || fail "$label does not point at the versioned runsc shim config"
}

secure_dir "$containerd_dir" 'K3s containerd directory'
secure_dir "$bin_parent" 'gVisor binary parent directory'
secure_dir "$links_parent" 'runtime link directory'
if [[ -e $bin_root || -L $bin_root ]]; then
  secure_dir "$bin_root" 'gVisor binary directory'
  for unmanaged_path in "$bin_root/runsc" "$bin_root/containerd-shim-runsc-v1" "$bin_root/gvisor-bin"; do
    [[ ! -e $unmanaged_path && ! -L $unmanaged_path ]] \
      || fail "unmanaged gVisor path already exists; refusing to overwrite: $unmanaged_path"
  done
fi
if [[ -e $version_dir || -L $version_dir ]]; then
  fail "version directory already exists; refusing to replace it: $version_dir"
fi
if [[ -e $runsc_link || -L $runsc_link ]]; then
  fail "refusing to replace existing runsc path: $runsc_link"
fi
if [[ -e $shim_link || -L $shim_link ]]; then
  fail "refusing to replace existing containerd shim path: $shim_link"
fi

template_exists=false
if [[ -e $template || -L $template ]]; then
  secure_file "$template" 'containerd template'
  template_exists=true
  if has_runsc_handler "$template"; then
    fail 'a runsc runtime stanza already exists in the containerd template; inspect it instead of overwriting it'
  fi
  assert_default_runc "$template" template
fi
secure_file "$rendered" 'rendered containerd config'
if has_runsc_handler "$rendered"; then
  fail 'a runsc runtime stanza already exists in the rendered containerd config; inspect it instead of overwriting it'
fi
assert_default_runc "$rendered" rendered

backup_parent=$(dirname "$backup_dir")
secure_dir "$backup_parent" 'backup parent directory'
if [[ -e $backup_dir || -L $backup_dir ]]; then
  secure_dir "$backup_dir" 'gVisor backup directory'
fi

cgroup_root=$(map_path /sys/fs/cgroup)
if [[ $test_mode == 1 && -n ${GVISOR_NODE_TEST_CGROUP_FS:-} ]]; then
  cgroup_fs=$GVISOR_NODE_TEST_CGROUP_FS
else
  mountpoint -q "$cgroup_root" || fail "cgroup filesystem is not mounted at $cgroup_root"
  cgroup_fs=$(stat -fc %T "$cgroup_root")
fi
case "$cgroup_fs" in
  cgroup2fs) cgroup_mode=v2 ;;
  tmpfs) cgroup_mode=v1-or-hybrid ;;
  *) fail "unsupported or unknown cgroup filesystem type for runsc configuration: $cgroup_fs" ;;
esac

archive_list_file=
archive_verbose_file=
extract_dir=
stage_dir=
template_stage=
backup_temp=
backup_path=
template_swapped=false
version_created=false
runsc_link_created=false
shim_link_created=false
restart_attempted=false

rollback() {
  local failed_status=$1 restore_stage rollback_failed=false
  [[ $failed_status -eq 0 ]] && return
  set +e

  if [[ $template_swapped == true ]]; then
    if [[ $template_exists == true ]]; then
      restore_stage=$(mktemp "$containerd_dir/.gvisor-restore.XXXXXX")
      if [[ -n $backup_path ]] && cp -- "$backup_path" "$restore_stage" \
        && chmod --reference="$backup_path" "$restore_stage" \
        && [[ ! -L $template ]] \
        && mv -T -- "$restore_stage" "$template"; then
        :
      else
        rollback_failed=true
        [[ -z ${restore_stage:-} ]] || rm -f -- "$restore_stage"
      fi
    elif [[ -f $template && ! -L $template ]]; then
      rm -f -- "$template" || rollback_failed=true
    else
      rollback_failed=true
    fi
  fi

  if [[ $shim_link_created == true ]]; then
    if [[ -L $shim_link && $(readlink -- "$shim_link") == "$version_dir/containerd-shim-runsc-v1" ]]; then
      rm -f -- "$shim_link" || rollback_failed=true
    else
      rollback_failed=true
    fi
  fi
  if [[ $runsc_link_created == true ]]; then
    if [[ -L $runsc_link && $(readlink -- "$runsc_link") == "$version_dir/runsc" ]]; then
      rm -f -- "$runsc_link" || rollback_failed=true
    else
      rollback_failed=true
    fi
  fi
  if [[ $version_created == true ]]; then
    if [[ -d $version_dir && ! -L $version_dir && $version_dir == "$bin_root/"* ]]; then
      rm -rf -- "$version_dir" || rollback_failed=true
    else
      rollback_failed=true
    fi
  fi

  if [[ $rollback_failed == true ]]; then
    warn 'installation failed and automatic file recovery was incomplete; inspect the timestamped backup and paths above'
  elif [[ $restart_attempted == true ]]; then
    warn 'installation failed; files were restored, but the service was not restarted a second time; restart it once in a maintenance window to load the restored template'
  else
    warn 'installation failed; changed files were restored and no service restart was attempted'
  fi
}

cleanup_exit() {
  local status=$?
  if (( status != 0 )); then
    rollback "$status"
  fi
  [[ -z ${backup_temp:-} ]] || rm -f -- "$backup_temp"
  [[ -z ${template_stage:-} ]] || rm -f -- "$template_stage"
  [[ -z ${stage_dir:-} ]] || rm -rf -- "$stage_dir"
  [[ -z ${extract_dir:-} ]] || rm -rf -- "$extract_dir"
  [[ -z ${archive_list_file:-} ]] || rm -f -- "$archive_list_file"
  [[ -z ${archive_verbose_file:-} ]] || rm -f -- "$archive_verbose_file"
  trap - EXIT
  exit "$status"
}
trap cleanup_exit EXIT

archive_list_file=$(mktemp)
tar --list --quoting-style=escape --file "$archive" > "$archive_list_file" \
  || fail 'could not read the supplied archive'
[[ -s $archive_list_file ]] || fail 'archive is empty'

declare -A archive_seen=()
runsc_member=
shim_member=
gvisor_dir_member=
sidecar_members=()
while IFS= read -r member; do
  [[ -n $member ]] || fail 'archive contains an empty member name'
  case "$member" in
    runsc)
      [[ -z ${archive_seen[runsc]+x} ]] || fail 'archive contains duplicate runsc members'
      archive_seen[runsc]=1
      runsc_member=$member
      ;;
    containerd-shim-runsc-v1)
      [[ -z ${archive_seen[shim]+x} ]] || fail 'archive contains duplicate containerd shim members'
      archive_seen[shim]=1
      shim_member=$member
      ;;
    gvisor-bin|gvisor-bin/)
      [[ -z ${archive_seen[gvisor-bin]+x} ]] || fail 'archive contains duplicate gvisor-bin directories'
      archive_seen[gvisor-bin]=1
      gvisor_dir_member=$member
      ;;
    gvisor-bin/*)
      sidecar=${member#gvisor-bin/}
      [[ $sidecar =~ ^[A-Za-z0-9._+-]+$ && $sidecar != . && $sidecar != .. && $sidecar != */* ]] \
        || fail "archive has an invalid gvisor-bin member layout: $member"
      sidecar_key="gvisor-bin/$sidecar"
      [[ -z ${archive_seen[$sidecar_key]+x} ]] \
        || fail "archive contains duplicate sidecar member: $member"
      archive_seen["$sidecar_key"]=1
      sidecar_members+=("$member")
      ;;
    *)
      fail "archive has an unexpected member or path traversal attempt: $member"
      ;;
  esac
done < "$archive_list_file"

[[ -n $runsc_member && -n $shim_member && -n $gvisor_dir_member ]] \
  || fail 'archive must contain exactly one runsc, containerd-shim-runsc-v1, and gvisor-bin directory'
(( ${#sidecar_members[@]} > 0 )) || fail 'archive gvisor-bin directory is empty'

archive_verbose_file=$(mktemp)
tar --list --verbose --quoting-style=escape --file "$archive" > "$archive_verbose_file" \
  || fail 'could not inspect archive metadata'

archive_type() {
  local member=$1 expected=$2 listing
  listing=$(awk -v wanted="$member" '
    $NF == wanted { print; count++ }
    END { exit(count == 1 ? 0 : 1) }
  ' "$archive_verbose_file") \
    || fail "archive member metadata is missing or ambiguous: $member"
  [[ $listing == "$expected"* ]] \
    || fail "archive member is not the expected regular file/directory type: $member"
}
archive_type "$runsc_member" '-'
archive_type "$shim_member" '-'
archive_type "$gvisor_dir_member" 'd'
for sidecar_member in "${sidecar_members[@]}"; do
  archive_type "$sidecar_member" '-'
done

extract_dir=$(mktemp -d)
tar --extract --file "$archive" --directory "$extract_dir" \
  --no-same-owner --no-same-permissions --keep-old-files \
  || fail 'could not safely extract the supplied archive'

[[ -f $extract_dir/runsc && ! -L $extract_dir/runsc && -x $extract_dir/runsc ]] \
  || fail 'archive runsc is not a regular executable file at the archive root'
[[ -f $extract_dir/containerd-shim-runsc-v1 && ! -L $extract_dir/containerd-shim-runsc-v1 \
  && -x $extract_dir/containerd-shim-runsc-v1 ]] \
  || fail 'archive containerd shim is not a regular executable file at the archive root'
[[ -d $extract_dir/gvisor-bin && ! -L $extract_dir/gvisor-bin ]] \
  || fail 'archive gvisor-bin is not a regular directory adjacent to runsc'
for sidecar_member in "${sidecar_members[@]}"; do
  sidecar=${sidecar_member#gvisor-bin/}
  sidecar_path=$extract_dir/gvisor-bin/$sidecar
  [[ -f $sidecar_path && ! -L $sidecar_path && -x $sidecar_path ]] \
    || fail "archive sidecar is not a regular executable file: $sidecar_member"
done

while IFS= read -r -d '' extracted_path; do
  extracted_member=${extracted_path#"$extract_dir"/}
  case "$extracted_member" in
    runsc|containerd-shim-runsc-v1|gvisor-bin|gvisor-bin/*) ;;
    *) fail "archive extraction produced an unexpected path: $extracted_member" ;;
  esac
done < <(find "$extract_dir" -mindepth 1 -print0)

if [[ -e $bin_root || -L $bin_root ]]; then
  secure_dir "$bin_root" 'gVisor binary directory'
else
  install -d -m 0755 -- "$bin_root"
fi
stage_dir=$(mktemp -d "$bin_root/.staging-${checksum}.XXXXXX")
chmod 0755 -- "$stage_dir"
install -m 0755 -- "$extract_dir/runsc" "$stage_dir/runsc"
install -m 0755 -- "$extract_dir/containerd-shim-runsc-v1" "$stage_dir/containerd-shim-runsc-v1"
install -d -m 0755 -- "$stage_dir/gvisor-bin"
for sidecar_member in "${sidecar_members[@]}"; do
  sidecar=${sidecar_member#gvisor-bin/}
  install -m 0755 -- "$extract_dir/gvisor-bin/$sidecar" "$stage_dir/gvisor-bin/$sidecar"
done
cat > "$stage_dir/runsc.toml" <<EOF
binary_name = "$version_dir_logical/runsc"
grouping = true

[runsc_config]
EOF
if [[ $cgroup_mode == v2 ]]; then
  cat >> "$stage_dir/runsc.toml" <<'EOF'

  systemd-cgroup = "true"
EOF
fi
if [[ -n $rootfs_memory_mib ]]; then
  printf '  overlay2 = "root:memory,size=%sm"\n' "$rootfs_memory_mib" >> "$stage_dir/runsc.toml"
fi
chmod 0644 -- "$stage_dir/runsc.toml"

validate_version_tree() {
  local root=$1 allow_config=$2 path relative
  [[ -d $root && ! -L $root ]] || fail "staged gVisor version is not a regular directory: $root"
  [[ -f $root/runsc && ! -L $root/runsc && -x $root/runsc ]] \
    || fail "staged runsc is not a regular executable: $root/runsc"
  [[ -f $root/containerd-shim-runsc-v1 && ! -L $root/containerd-shim-runsc-v1 \
    && -x $root/containerd-shim-runsc-v1 ]] \
    || fail "staged containerd shim is not a regular executable: $root/containerd-shim-runsc-v1"
  [[ -d $root/gvisor-bin && ! -L $root/gvisor-bin ]] \
    || fail "staged gvisor-bin is not a regular directory: $root/gvisor-bin"
  if [[ $allow_config == true ]]; then
    [[ -f $root/runsc.toml && ! -L $root/runsc.toml ]] \
      || fail "staged runsc shim config is missing or unsafe: $root/runsc.toml"
  fi
  while IFS= read -r -d '' path; do
    relative=${path#"$root"/}
    case "$relative" in
      runsc|containerd-shim-runsc-v1|runsc.toml|gvisor-bin) ;;
      gvisor-bin/*)
        [[ ${relative#gvisor-bin/} != */* ]] || fail "staged gvisor-bin is nested: $path"
        [[ -f $path && ! -L $path && -x $path ]] || fail "staged sidecar is not a regular executable: $path"
        ;;
      *) fail "staged gVisor version has an unexpected path: $path" ;;
    esac
  done < <(find "$root" -mindepth 1 -print0)
}
validate_version_tree "$stage_dir" true

mv -T --no-clobber -- "$stage_dir" "$version_dir" \
  || fail "could not atomically publish gVisor version directory: $version_dir"
[[ ! -e $stage_dir && ! -L $stage_dir && -d $version_dir && ! -L $version_dir ]] \
  || fail "version directory publication raced with another change: $version_dir"
stage_dir=
version_created=true

ln -s -- "$version_dir/runsc" "$runsc_link" || fail "could not create runsc link: $runsc_link"
runsc_link_created=true
ln -s -- "$version_dir/containerd-shim-runsc-v1" "$shim_link" \
  || fail "could not create containerd shim link: $shim_link"
shim_link_created=true

if [[ -e $backup_dir || -L $backup_dir ]]; then
  secure_dir "$backup_dir" 'gVisor backup directory'
else
  install -d -m 0700 -- "$backup_dir"
fi
if [[ $template_exists == true ]]; then
  backup_path=$backup_dir/config-v3.toml.tmpl.$(date -u +%Y%m%dT%H%M%SZ).$$
  backup_temp=$(mktemp "$backup_dir/.backup.XXXXXX")
  cp -- "$template" "$backup_temp"
  chmod --reference="$template" "$backup_temp"
  mv -T --no-clobber -- "$backup_temp" "$backup_path" \
    || fail "could not publish the containerd template backup: $backup_path"
  backup_temp=
fi

template_stage=$(mktemp "$containerd_dir/.gvisor-template.XXXXXX")
if [[ $template_exists == true ]]; then
  cp -- "$template" "$template_stage"
  chmod --reference="$template" "$template_stage"
else
  cat > "$template_stage" <<'EOF'
{{ template "base" . }}
EOF
  chmod 0644 -- "$template_stage"
fi
cat >> "$template_stage" <<EOF

[plugins.'io.containerd.cri.v1.runtime'.containerd.runtimes.runsc]
  runtime_type = "io.containerd.runsc.v1"
[plugins.'io.containerd.cri.v1.runtime'.containerd.runtimes.runsc.options]
  TypeUrl = "io.containerd.runsc.v1.options"
  ConfigPath = "$runsc_config_logical"
EOF
assert_handler_file "$template_stage" 'new containerd template' "$runsc_config_logical"
mv -T -- "$template_stage" "$template" \
  || fail "could not atomically publish the containerd template: $template"
template_stage=
template_swapped=true

if systemctl is-active --quiet k3s; then
  service=k3s
elif systemctl is-active --quiet k3s-agent; then
  service=k3s-agent
else
  fail 'neither k3s nor k3s-agent is active after preflight'
fi
printf 'Registered runsc in %s. Restarting %s once because --restart-k3s was explicitly requested.\n' \
  "$template_logical" "$service"
restart_attempted=true
systemctl restart "$service" || fail "$service restart failed; the template will be restored without an automatic second restart"
systemctl is-active --quiet "$service" \
  || fail "$service did not return active after the requested restart"

secure_file "$rendered" 'rendered containerd config after restart'
assert_default_runc "$rendered" 'rendered config after restart'
assert_handler_file "$rendered" 'rendered containerd config' "$runsc_config_logical"
plugins_output=$(k3s ctr plugins ls 2>/dev/null) \
  || fail 'containerd did not answer k3s ctr plugins ls after restart'
if ! awk 'tolower($0) ~ /cri/ && tolower($0) ~ /(^|[[:space:]])ok([[:space:]]|$)/ { found=1 } END { exit(found ? 0 : 1) }' \
  <<< "$plugins_output"; then
  fail 'containerd CRI plugin was not reported healthy after restart'
fi

printf 'PASS: REGISTERED runsc handler; default runtime remains runc; containerd service and CRI plugin are healthy after one requested restart.\n'
printf 'NOT ACCEPTANCE: this script did not start a workload or prove gVisor compatibility. Run a disposable non-privileged RuntimeClass canary and compare it with runc before labeling or production use.\n'
printf 'BACKUP: %s\n' "${backup_path:-none (template was newly created)}"
printf 'NOTE: --node matched local hostname %s. This script did not query Kubernetes; independently verify any different Kubernetes Node name mapping.\n' "$local_hostname"
