#!/bin/bash
# Sourced only in the runsc matrix's disposable mount namespace.
mkdir -p "$base/gvisor" "$base/runsc-state"
curl --fail --location --retry 3 'https://github.com/google/gvisor/releases/download/release-20260831.0/gvisor-x86_64.tar.bz2' -o "$base/gvisor.tar.bz2"
echo "014b3871a5c698c802fd7a03758e0dbf4c1683f9e3f8c743979ea66bbf6553a4  $base/gvisor.tar.bz2" | sha256sum --check
# Keep runsc, shim and all adjacent gvisor-bin sidecars from the exact archive.
tar -xjf "$base/gvisor.tar.bz2" -C "$base/gvisor"
export PATH="$base/gvisor:$PATH"
"$base/gvisor/runsc" --version | tee "$artifacts/runsc-version.txt"
grep -F 'release-20260831.0' "$artifacts/runsc-version.txt"
find "$base/gvisor" -maxdepth 2 -type f -printf '%P\n' > "$artifacts/gvisor-payload-files.txt"
test -d "$base/gvisor/gvisor-bin"
jq --arg runtime "$base/gvisor/runsc" '.real_runtime=$runtime' /etc/mwc-runtime-wrapper.json > /etc/mwc-runsc-wrapper.json
chmod 0600 /etc/mwc-runsc-wrapper.json
# External CI observation, not a wrapper test flag. Check the child overlay is
# still mounted between real runsc create and start, before its Gofer exists.
cat > "$base/observe-runsc" <<'SCRIPT'
#!/bin/bash
set -euo pipefail
base=/var/lib/mwc-runtime-prototype
bundle_next=0
for arg in "$@"; do
 if [ "$bundle_next" = 1 ]; then cp "$arg/config.json" "$base/pre-wrapper-${@: -1}-oci.json"; bundle_next=0; fi
 if [ "$arg" = --bundle ]; then bundle_next=1; fi
 if [ "$arg" = start ]; then
  id="${@: -1}"
  test "$(jq -r .phase "$base/state/$id.json")" = created
  grep -F "upperdir=$(jq -r .upper "$base/state/$id.json")" /proc/self/mountinfo > "$base/start-$id-mount.txt"
  cp "$base/state/$id.json" "$base/start-$id-record.json"
 fi
done
exec "$base/mwc-runsc-wrapper" "$@"
SCRIPT
chmod 0755 "$base/observe-runsc"
cat > "$base/runsc.toml" <<TOML
binary_name = "$base/observe-runsc"
root = "$base/runsc-state"
grouping = true
[runsc_config]
  systemd-cgroup = "false"
  platform = "systrap"
  overlay2 = "none"
  file-access = "shared"
  file-access-mounts = "shared"
  oci-seccomp = "true"
  gvisor-marker-file = "true"
TOML
cat >> "$base/containerd.toml" <<TOML
    [plugins.'io.containerd.cri.v1.runtime'.containerd.runtimes.'mwc-runsc']
      runtime_type = 'io.containerd.runsc.v1'
      container_annotations = ['dev.gvisor.*']
      [plugins.'io.containerd.cri.v1.runtime'.containerd.runtimes.'mwc-runsc'.options]
        ConfigPath = '$base/runsc.toml'
TOML
# Conflicting storage policy is supplied through the actual shim ConfigPath.
sed 's/overlay2 = "none"/overlay2 = "root:memory"/' "$base/runsc.toml" > "$base/runsc-conflict.toml"
cat >> "$base/containerd.toml" <<TOML
    [plugins.'io.containerd.cri.v1.runtime'.containerd.runtimes.'mwc-runsc-conflict']
      runtime_type = 'io.containerd.runsc.v1'
      [plugins.'io.containerd.cri.v1.runtime'.containerd.runtimes.'mwc-runsc-conflict'.options]
        ConfigPath = '$base/runsc-conflict.toml'
TOML
cp "$base/runsc.toml" "$artifacts/runsc-shim-config.toml"
