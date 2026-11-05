#!/bin/bash
# Only an expendable CI VM. No synthetic CSI publication is permitted.
set -euo pipefail
: "${GITHUB_ACTIONS:?Disposable CI VM only}"
inputs="${MWC_PRODUCT_STORAGE_INPUTS:?Verified offline CSI inputs required}"
copy_input() {
 local name="$1" digest="$2" destination="$3"
 cp "$inputs/$name" "$destination"
 printf '%s  %s\n' "$digest" "$destination" | sha256sum -c -
}
: "${RUNNER_TEMP:?}"
: "${GITHUB_WORKSPACE:?}"
base=/var/lib/mwc-product-storage-ci
out="$GITHUB_WORKSPACE/artifacts"
fixtures="$RUNNER_TEMP/product-storage-fixtures"
namespace=memeloop-workspace-control
mkdir -p "$base" "$out" "$base/state/mwc-native" "$base/state/mwc-runsc"
chmod 0700 "$base/state" "$base/state/"*
exec > >(tee "$out/product-csi.log") 2>&1
k3s_pid=''; containerd_pid=''; loops=()
kube() { KUBECONFIG="$base/kubeconfig" "$base/k3s-bin" kubectl "$@"; }
cri() { crictl --runtime-endpoint "unix://$base/containerd.sock" --image-endpoint "unix://$base/containerd.sock" "$@"; }
ctr_ci() { ctr --address "$base/containerd.sock" --namespace k8s.io "$@"; }
snapshot() {
 kube get pods,pvc -A -o json > "$out/final-pods-pvc.json" 2>/dev/null || true
 kube get pv,logicalvolumes.topolvm.io -o json > "$out/final-pv-lv.json" 2>/dev/null || true
 kube get events -A -o json > "$out/events.json" 2>/dev/null || true
 kube logs -n topolvm-system -l app.kubernetes.io/component=node --all-containers --prefix > "$out/topolvm-node.log" 2>&1 || true
 kube logs -n topolvm-system -l app.kubernetes.io/component=controller --all-containers --prefix > "$out/topolvm-controller.log" 2>&1 || true
 kube logs -n topolvm-system -l app.kubernetes.io/component=lvmd --all-containers --prefix > "$out/topolvm-lvmd.log" 2>&1 || true
 cat /proc/self/mountinfo > "$out/final-mountinfo.txt"
 lvs -a -o lv_name,vg_name,lv_uuid,lv_attr,pool_lv,data_percent,metadata_percent --reportformat json > "$out/final-lvm.json" || true
 if [ -d "$base/state" ]; then cp -a "$base/state" "$out/wrapper-state"; fi
}
finish() {
 status=$?
 trap - EXIT
 snapshot
 if [ -n "$k3s_pid" ]; then kill "$k3s_pid" 2>/dev/null || true; wait "$k3s_pid" || true; fi
 if [ -n "$containerd_pid" ]; then kill "$containerd_pid" 2>/dev/null || true; wait "$containerd_pid" || true; fi
 # Never detach a mounted or busy LV to turn cleanup failure into a pass.
 for vg in mwc-ci-home mwc-ci-scratch; do
  if vgs "$vg" >/dev/null 2>&1; then
   if ! vgremove -y "$vg" >> "$out/cleanup.log" 2>&1; then status=1; fi
  fi
 done
 for loop in "${loops[@]}"; do
  if ! losetup -d "$loop" >> "$out/cleanup.log" 2>&1; then status=1; fi
 done
 jq -n --argjson exit_status "$status" '{exit_status:$exit_status,normal_cleanup:($exit_status==0)}' > "$out/cleanup-receipt.json"
 if [ "$status" != 0 ] && [ -f "$out/acceptance.json" ]; then
  jq '.accepted=false | .cleanup_failed=true' "$out/acceptance.json" > "$out/acceptance-updated.json"
  mv "$out/acceptance-updated.json" "$out/acceptance.json"
 fi
 exit "$status"
}

# The offline Harbor consumer supplies the actual digest-checked metadata and
# OCI content. No external registry/download execution entrypoint is supported.
(cd "$inputs" && sha256sum -c SHA256SUMS)
cp "$inputs/registry/"*.json "$out/"
trap finish EXIT

copy_input k3s \
 65a55ec56c24eab44383086166ec620a491952b7e23941a49ddca6e8a4c4b4de "$base/k3s-bin"
chmod 0755 "$base/k3s-bin"
K3S_DATA_DIR="$base/k3s" "$base/k3s-bin" ctr version --help >/dev/null
export PATH="$base/k3s/data/current/bin:$PATH"
containerd --version | tee "$out/containerd-version.txt"
runc --version | tee "$out/runc-version.txt"
grep -F 'v2.3.2-k3s2' "$out/containerd-version.txt"
grep -Fx 'runc version 1.4.2' "$out/runc-version.txt"
uname -a > "$out/kernel.txt"
install -m 0755 "$RUNNER_TEMP/product-storage-bin/mwc-runtime-wrapper" "$base/mwc-runtime-wrapper"
install -m 0755 "$RUNNER_TEMP/product-storage-bin/mwc-runsc-wrapper" "$base/mwc-runsc-wrapper"
mkdir -p "$base/gvisor" "$base/runsc-state"
copy_input gvisor.tar.bz2 \
 014b3871a5c698c802fd7a03758e0dbf4c1683f9e3f8c743979ea66bbf6553a4 "$base/gvisor.tar.bz2"
tar -xjf "$base/gvisor.tar.bz2" -C "$base/gvisor"
export PATH="$base/gvisor:$PATH"
"$base/gvisor/runsc" --version | tee "$out/runsc-version.txt"
grep -F release-20260831.0 "$out/runsc-version.txt"
test -x "$base/gvisor/containerd-shim-runsc-v1"
test -d "$base/gvisor/gvisor-bin"
find "$base/gvisor" -type f -printf '%P\n' > "$out/gvisor-payload-files.txt"
for kind in home scratch; do
 if [ "$kind" = home ]; then backing=14G; pool=12G; else backing=7G; pool=6G; fi
 truncate -s "$backing" "$base/$kind.img"
 loop="$(losetup --find --show "$base/$kind.img")"; loops+=("$loop")
 pvcreate -y "$loop"
 vgcreate "mwc-ci-$kind" "$loop"
 lvcreate -L "$pool" --poolmetadatasize 32M -T "mwc-ci-$kind/pool"
done
vg_uuid="$(vgs --noheadings -o vg_uuid mwc-ci-scratch | tr -d '[:space:]-')"
pool_uuid="$(dmsetup info -c --noheadings -o uuid mwc--ci--scratch-pool-tpool | xargs)"
jq -n --arg runtime "$(readlink -f "$(command -v runc)")" --arg vg "$vg_uuid" --arg pool "$pool_uuid" --arg dmsetup "$(readlink -f "$(command -v dmsetup)")" --arg base "$base" \
 '{real_runtime:$runtime,kubelet_root:"/var/lib/kubelet",cri_root:($base+"/containerd-root/io.containerd.grpc.v1.cri"),cri_state_root:($base+"/containerd-state/io.containerd.grpc.v1.cri"),workspace_namespace:"memeloop-workspace-control",pod_log_root:"/var/log/pods",bundle_root:($base+"/containerd-state/io.containerd.runtime.v2.task/k8s.io"),snapshot_root:($base+"/containerd-root/io.containerd.snapshotter.v1.overlayfs/snapshots"),state_root:($base+"/state/mwc-native"),mount_namespace:"/proc/1/ns/mnt",scratch_vg_uuid:$vg,scratch_pool_dm_uuid:$pool,dmsetup:$dmsetup}' > /etc/mwc-runtime-wrapper.json
chmod 0600 /etc/mwc-runtime-wrapper.json
jq --arg runtime "$base/gvisor/runsc" --arg state "$base/state/mwc-runsc" '.real_runtime=$runtime | .state_root=$state' /etc/mwc-runtime-wrapper.json > /etc/mwc-runsc-wrapper.json
chmod 0600 /etc/mwc-runsc-wrapper.json
cat > "$base/runsc.toml" <<TOML
binary_name = '$base/mwc-runsc-wrapper'
root = '$base/runsc-state'
grouping = true
[runsc_config]
  systemd-cgroup = 'true'
  platform = 'systrap'
  overlay2 = 'none'
  file-access = 'shared'
  file-access-mounts = 'shared'
  oci-seccomp = 'true'
  gvisor-marker-file = 'true'
TOML
cp "$base/runsc.toml" "$out/runsc-shim-config.toml"
cat > "$base/containerd.toml" <<TOML
version = 3
root = '$base/containerd-root'
state = '$base/containerd-state'
[grpc]
  address = '$base/containerd.sock'
[plugins.'io.containerd.cri.v1.images']
  snapshotter = 'overlayfs'
  [plugins.'io.containerd.cri.v1.images'.pinned_images]
    sandbox = 'registry.k8s.io/pause:3.10'
[plugins.'io.containerd.cri.v1.runtime'.cni]
  bin_dirs = ['$base/k3s/data/current/bin']
  conf_dir = '$base/k3s/agent/etc/cni/net.d'
[plugins.'io.containerd.cri.v1.runtime'.containerd]
  default_runtime_name = 'runc'
  [plugins.'io.containerd.cri.v1.runtime'.containerd.runtimes.runc]
    runtime_type = 'io.containerd.runc.v2'
    [plugins.'io.containerd.cri.v1.runtime'.containerd.runtimes.runc.options]
      SystemdCgroup = true
  [plugins.'io.containerd.cri.v1.runtime'.containerd.runtimes.mwc-native]
    runtime_type = 'io.containerd.runc.v2'
    [plugins.'io.containerd.cri.v1.runtime'.containerd.runtimes.mwc-native.options]
      BinaryName = '$base/mwc-runtime-wrapper'
      SystemdCgroup = true
  [plugins.'io.containerd.cri.v1.runtime'.containerd.runtimes.mwc-runsc]
    runtime_type = 'io.containerd.runsc.v1'
    [plugins.'io.containerd.cri.v1.runtime'.containerd.runtimes.mwc-runsc.options]
      ConfigPath = '$base/runsc.toml'
TOML
containerd --config "$base/containerd.toml" > "$out/containerd.log" 2>&1 & containerd_pid=$!
for n in $(seq 1 60); do if cri info > "$out/cri-info.json" 2>/dev/null; then break; fi; sleep 1; done
cri info > "$out/cri-info.json"
# Imports and kubelet use one real containerd content store. Stage archives
# contain exact original references, so the disconnected guest never pulls.
ctr_ci images import --no-unpack --digests "$inputs/images.tar"
# The guest owns this input disk. Release the now-duplicated archive before
# image unpack and the real four-GiB ENOSPC case; discard reaches the job file.
rm "$inputs/images.tar"
sync
fstrim -v "$inputs" > "$out/input-disk-trim.txt"
# Keep actual production limits and security contexts. Ubuntu's VM policy must
# permit the existing unprivileged RootlessKit user namespace, as main CI does.
if [ -e /proc/sys/kernel/apparmor_restrict_unprivileged_userns ]; then sysctl -w kernel.apparmor_restrict_unprivileged_userns=0; fi
K3S_DATA_DIR="$base/k3s" "$base/k3s-bin" server --data-dir "$base/k3s" --write-kubeconfig "$base/kubeconfig" \
 --node-name mwc-product-ci --container-runtime-endpoint "unix://$base/containerd.sock" \
 --disable traefik,servicelb,local-storage,metrics-server --disable-helm-controller \
 > "$out/k3s.log" 2>&1 & k3s_pid=$!
for n in $(seq 1 150); do
 if kube get node mwc-product-ci >/dev/null 2>&1; then break; fi
 if ! kill -0 "$k3s_pid"; then tail -n 40 "$out/k3s.log"; exit 1; fi
 sleep 1
done
kube wait --for=condition=Ready node/mwc-product-ci --timeout=120s
kube apply -f - <<'YAML'
apiVersion: node.k8s.io/v1
kind: RuntimeClass
metadata: {name: mwc-native}
handler: mwc-native
---
apiVersion: node.k8s.io/v1
kind: RuntimeClass
metadata: {name: mwc-runsc}
handler: mwc-runsc
YAML
copy_input helm.tgz \
 c306b46f719b0a4da32d0f78ee21bf90ce8d602f15b22ab753f0674d1670a7f3 "$base/helm.tgz"
tar -xzf "$base/helm.tgz" -C "$base"
copy_input topolvm.tgz \
 4b6c070da0b38f098c9ef6b6a38ef4e2dfff52adecedc280ad92bfab408f90ee "$base/topolvm.tgz"
cat > "$base/topolvm-values.yaml" <<'YAML'
cert-manager: {enabled: false}
webhook:
  certManager: false
  podMutatingWebhook: {enabled: false}
scheduler: {enabled: false}
controller: {replicaCount: 1}
image:
  repository: ghcr.io/topolvm/topolvm-with-sidecar
  reference: 0.41.1@sha256:70548dbe0c6addcccf79a557f29e95db2e6dc2cba2102988c91f30086004d0fc
lvmd:
  deviceClasses:
    - {name: mwc-ci-home, volume-group: mwc-ci-home, type: thin, spare-gb: 0, thin-pool: {name: pool, overprovision-ratio: 2.0}}
    - {name: mwc-ci-scratch, volume-group: mwc-ci-scratch, type: thin, default: true, spare-gb: 0, thin-pool: {name: pool, overprovision-ratio: 2.0}}
storageClasses:
  - name: mwc-ci-home
    storageClass: {fsType: xfs, reclaimPolicy: Delete, volumeBindingMode: WaitForFirstConsumer, additionalParameters: {topolvm.io/device-class: mwc-ci-home}}
  - name: mwc-ci-scratch
    storageClass: {fsType: xfs, reclaimPolicy: Delete, volumeBindingMode: WaitForFirstConsumer, additionalParameters: {topolvm.io/device-class: mwc-ci-scratch}}
YAML
KUBECONFIG="$base/kubeconfig" "$base/linux-amd64/helm" install topolvm "$base/topolvm.tgz" -n topolvm-system --create-namespace -f "$base/topolvm-values.yaml" --wait --timeout 5m
kube get storageclasses,csidrivers,csinodes -o json > "$out/actual-csi-installation.json"

for runtime in mwc-native mwc-runsc; do
for enabled in false true; do
 case_id="$runtime-buildkit-$enabled"
 fixture="$fixtures/$case_id.json"
 case_out="$out/$case_id"; mkdir -p "$case_out"
 jq -e --arg runtime "$runtime" --argjson enabled "$enabled" '.pod.spec.runtimeClassName==$runtime and .scope.runtime==$runtime and .scope.buildkit==$enabled and .scope.scratch_gib>=4 and .scratch.spec.resources.requests.storage=="4Gi" and all((.pod.spec.containers + (.pod.spec.initContainers // []))[]; (.resources.limits // {} | has("ephemeral-storage") | not))' "$fixture"
 pod="$(jq -r '.names.pod' "$fixture")"
 home_claim="$(jq -r '.names.home' "$fixture")"
 scratch_claim="$(jq -r '.names.scratch' "$fixture")"
 # These are precisely the ResourceBuilder's real config/projections/services.
 jq '{apiVersion:"v1",kind:"List",items:([.namespace,.scratch,.service,.config]+.injections)}' "$fixture" | kube apply -f -
 identity="$(jq -r '.identity.metadata.name' "$fixture")"
 ssh-keygen -q -t ed25519 -N '' -f "$base/host-$case_id"
 kube create secret generic -n "$namespace" "$identity" \
  --from-file="ssh_host_ed25519_key=$base/host-$case_id" --from-file="ssh_host_ed25519_key.pub=$base/host-$case_id.pub"
 jq '.statefulset' "$fixture" | kube apply -f -
 kube wait -n "$namespace" --for=condition=Ready "pod/$pod" --timeout=10m
 kube get pod -n "$namespace" "$pod" -o json > "$case_out/pod-initial.json"
 uid="$(jq -r '.metadata.uid' "$case_out/pod-initial.json")"
 scratch_uid="$(kube get pvc -n "$namespace" "$scratch_claim" -o jsonpath='{.metadata.uid}')"
 home_uid="$(kube get pvc -n "$namespace" "$home_claim" -o jsonpath='{.metadata.uid}')"
 scratch_pv="$(kube get pvc -n "$namespace" "$scratch_claim" -o jsonpath='{.spec.volumeName}')"
 home_pv="$(kube get pvc -n "$namespace" "$home_claim" -o jsonpath='{.spec.volumeName}')"
 kube get pv "$scratch_pv" "$home_pv" -o json > "$case_out/actual-pvs.json"
 jq -e 'all(.items[];.spec.csi.driver=="topolvm.io" and .status.phase=="Bound")' "$case_out/actual-pvs.json"
 publish="/var/lib/kubelet/pods/$uid/volumes/kubernetes.io~csi/$scratch_pv/mount"
 test -f "$(dirname "$publish")/vol_data.json"
 cp "$(dirname "$publish")/vol_data.json" "$case_out/actual-csi-vol-data.json"
 findmnt -J -T "$publish" > "$case_out/actual-publish-mount.json"
 test "$(findmnt -n -o FSTYPE -T "$publish")" = xfs
 for id in $(jq -r '[.status.containerStatuses[],.status.initContainerStatuses[]?]|.[].containerID|sub("containerd://";"")' "$case_out/pod-initial.json"); do
  cri inspect "$id" > "$case_out/container-$id.json"
  cp "$base/containerd-state/io.containerd.runtime.v2.task/k8s.io/$id/config.json" "$case_out/oci-$id.json" 2>/dev/null || true
 done
 # Actual CRI imageID/repoDigest and OCI configs are preserved, not merely the
 # declared images. No product image is built or replaced in this workflow.
 for image in $(jq -r '[.pod.spec.containers[],.pod.spec.initContainers[]?]|.[].image' "$fixture" | sort -u); do
  cri inspecti "$image" > "$case_out/image-$(printf '%s' "$image" | sha256sum | cut -c1-12).json"
 done
 kube exec -n "$namespace" "$pod" -c workspace -- sh -ec 'rustc --version; cargo --version; printf home-survives > /home/user/storage-ci-home-marker; stat -c "%d %n" /home/user /tmp /var/tmp /var/lib/mwc/build-scratch; df -B1 / /home/user /tmp' > "$case_out/product-tools-storage.txt"
 # Use the original platform-created authorized client key; only this disposable
 # fixture's private key crosses to the runner, and it is never an artifact.
 kube exec -n "$namespace" "$pod" -c workspace -- cat /run/mwc-ssh/ttyd_client_key > "$base/client-$case_id"
 chmod 0600 "$base/client-$case_id"
 address="$(jq -r '.status.podIP' "$case_out/pod-initial.json")"
 printf '[%s]:2222 %s\n' "$address" "$(cut -d ' ' -f 1,2 "$base/host-$case_id.pub")" > "$base/known-$case_id"
 ssh_product() {
  ssh -F /dev/null -p 2222 -o BatchMode=yes -o StrictHostKeyChecking=yes -o ConnectTimeout=5 \
   -o "UserKnownHostsFile=$base/known-$case_id" -i "$base/client-$case_id" "user@$address" "$@"
 }
 ssh_product 'test "$(cat /home/user/storage-ci-home-marker)" = home-survives; printf strict-ssh-ok' > "$case_out/strict-ssh-before.txt"
 ttyd_path="$(jq -r '.pod.spec.containers[]|select(.name=="ttyd")|.args as $a|$a[($a|index("--base-path"))+1]' "$fixture")"
 curl -fsSL "http://$address:7681${ttyd_path}/" > "$case_out/ttyd-http.html"
 if [ "$enabled" = true ]; then
  # Actual rootless daemon security context, worker and layer generation.
  ssh_product 'mkdir -p /home/user/storage-ci-build; printf "FROM scratch\nCOPY marker /marker\n" > /home/user/storage-ci-build/Dockerfile; printf layer-proof > /home/user/storage-ci-build/marker; buildctl build --frontend dockerfile.v0 --local context=/home/user/storage-ci-build --local dockerfile=/home/user/storage-ci-build --output type=oci,dest=/home/user/storage-ci-layer.tar' > "$case_out/actual-buildkit-build.txt" 2>&1
 fi
 # A normal sidecar stop/delete must not remove another container's upper.
 main_id="$(jq -r '.status.containerStatuses[]|select(.name=="workspace")|.containerID|sub("containerd://";"")' "$case_out/pod-initial.json")"
 tty_id="$(jq -r '.status.containerStatuses[]|select(.name=="ttyd")|.containerID|sub("containerd://";"")' "$case_out/pod-initial.json")"
 cri stop "$tty_id"
 cri rm "$tty_id"
 for n in $(seq 1 90); do
  replacement_tty="$(kube get pod -n "$namespace" "$pod" -o json | jq -r '.status.containerStatuses[]|select(.name=="ttyd" and .ready)|.containerID|sub("containerd://";"")')"
  if [ -n "$replacement_tty" ] && [ "$replacement_tty" != "$tty_id" ]; then break; fi
  sleep 1
 done
 test -n "$replacement_tty"; test "$replacement_tty" != "$tty_id"
 test "$(kube get pod -n "$namespace" "$pod" -o json | jq -r '.status.containerStatuses[]|select(.name=="workspace")|.containerID|sub("containerd://";"")')" = "$main_id"
 ssh_product 'test "$(cat /home/user/storage-ci-home-marker)" = home-survives; printf main-survives-sidecar-delete' > "$case_out/strict-ssh-sidecar-replacement.txt"
 active_ids="$(kube get pod -n "$namespace" "$pod" -o json | jq '[.status.containerStatuses[].containerID|sub("containerd://";"")]')"
 jq -es --arg uid "$uid" --arg publish "$publish" --argjson ids "$active_ids" '
  [.[]|select(.pod_uid==$uid and (.id as $id|$ids|index($id)!=null))] |
  if length==($ids|length) and length>=2 and all(.[];.phase!="deleted" and .scratch.path==$publish and (.upper|startswith($publish+"/runtime-private/")) and all(.managed_files[]?;.redirected|startswith($publish+"/runtime-private/"))) then . else error("incorrect actual container scratch budget") end' "$base/state/$runtime/"*.json > "$case_out/actual-wrapper-records.json"
 for upper in $(jq -r '.[].upper' "$case_out/actual-wrapper-records.json"); do
  test "$(stat -Lc %d "$upper")" = "$(stat -Lc %d "$publish")"
 done
 kube exec -i -n "$namespace" "$pod" -c workspace -- sh -ec 'cat > /usr/local/bin/storage-ci-probe; chmod 0755 /usr/local/bin/storage-ci-probe' < "$RUNNER_TEMP/product-storage-bin/mwc-budget-probe"
 pod_logs="/var/log/pods/${namespace}_${pod}_${uid}"
 test -L "$pod_logs"
 test "$(readlink "$pod_logs")" = "$publish/runtime-private/logs/$uid"
 test "$(stat -Lc %d "$pod_logs")" = "$(stat -Lc %d "$publish")"
 findmnt -J -T "$pod_logs" > "$case_out/logs-shared-mount.json"
 summary() { kube get --raw /api/v1/nodes/mwc-product-ci/proxy/stats/summary; }
 check_identity() {
  kube get pod -n "$namespace" "$pod" -o json > "$case_out/pod-current.json"
  jq -e --arg uid "$uid" --arg id "containerd://$main_id" '.metadata.uid==$uid and .status.phase=="Running" and all(.status.containerStatuses[];.ready) and any(.status.containerStatuses[];.name=="workspace" and .containerID==$id)' "$case_out/pod-current.json" >/dev/null
 }
 # Use the real application PID 1 stdout, not kubectl exec's output stream.
 kube_log_emit() { timeout 90s "$base/k3s-bin" kubectl --kubeconfig "$base/kubeconfig" exec -n "$namespace" "$pod" -c workspace -- /usr/local/bin/storage-ci-probe emit /proc/1/fd/1 "$1" "$2"; }
 summary > "$case_out/summary-before-logs.json"
 timeout 240s "$base/k3s-bin" kubectl --kubeconfig "$base/kubeconfig" logs -n "$namespace" "$pod" -c workspace --follow --since=1s \
  > >(awk 'match($0,/MWC_LOG_[A-Z_]+/) {tag=substr($0,RSTART,RLENGTH); if (!seen[tag]++) {print tag; fflush()}}' > "$case_out/log-follow-markers.txt") & follow_pid=$!
 sleep 1
 kube_log_emit 67108864 MWC_LOG_BURST > "$case_out/log-burst.json"
 for n in $(seq 1 90); do
  if [ "$(find "$pod_logs/workspace" -maxdepth 1 -type f | wc -l)" -ge 2 ]; then break; fi
  check_identity
  sleep 2
 done
 test "$(find "$pod_logs/workspace" -maxdepth 1 -type f | wc -l)" -ge 2
 kube_log_emit 65536 MWC_LOG_REOPEN > "$case_out/log-reopen.json"
 logs_accounted=false
 for n in $(seq 1 30); do
  if grep -q MWC_LOG_REOPEN "$case_out/log-follow-markers.txt"; then break; fi
  sleep 1
 done
 grep -q MWC_LOG_BURST "$case_out/log-follow-markers.txt"
 grep -q MWC_LOG_REOPEN "$case_out/log-follow-markers.txt"
 kill "$follow_pid" 2>/dev/null || true
 wait "$follow_pid" 2>/dev/null || true
 for n in $(seq 1 30); do
  summary > "$case_out/summary-after-logs.json"
  if jq -e --arg uid "$uid" --slurpfile before "$case_out/summary-before-logs.json" '
   .pods[] | select(.podRef.uid==$uid) | . as $pod |
   .containers[] | select(.name=="workspace") | .logs.usedBytes as $logs |
   ($before[0].pods[] | select(.podRef.uid==$uid) | .containers[] | select(.name=="workspace") | .logs.usedBytes // 0) as $old |
   $logs>$old and $logs>1048576 and $pod["ephemeral-storage"].usedBytes >= $logs' "$case_out/summary-after-logs.json" >/dev/null; then logs_accounted=true; break; fi
  sleep 2
 done
 test "$logs_accounted" = true
 find -H "$pod_logs" -type f -printf '%P %s\n' > "$case_out/log-rotation-files.txt"
 test "$(du --summarize --block-size=1 --dereference "$pod_logs" | cut -f1)" -le 100663296
 kube_log_emit 65536 MWC_LOG_PREVIOUS > "$case_out/log-previous-emission.json"
 cri stop "$main_id"
 # Kubelet owns recreation and previous-log bookkeeping; do not delete its log.
 for n in $(seq 1 90); do
  new_main="$(kube get pod -n "$namespace" "$pod" -o json | jq -r '.status.containerStatuses[]|select(.name=="workspace" and .ready)|.containerID|sub("containerd://";"")')"
  if [ -n "$new_main" ] && [ "$new_main" != "$main_id" ]; then break; fi
  sleep 2
 done
 test -n "$new_main"; test "$new_main" != "$main_id"
 main_id="$new_main"
 check_identity
 kube logs -n "$namespace" "$pod" -c workspace --previous --limit-bytes=100663296 | awk '/MWC_LOG_PREVIOUS/ && !seen++ {print substr($0,1,180)}' > "$case_out/log-previous-markers.txt"
 grep -q MWC_LOG_PREVIOUS "$case_out/log-previous-markers.txt"
 ssh_product 'test "$(cat /home/user/storage-ci-home-marker)" = home-survives; printf strict-ssh-logs-ok' > "$case_out/strict-ssh-logs.txt"
 kube exec -i -n "$namespace" "$pod" -c workspace -- sh -ec 'cat > /usr/local/bin/storage-ci-probe; chmod 0755 /usr/local/bin/storage-ci-probe' < "$RUNNER_TEMP/product-storage-bin/mwc-budget-probe"
 timeout 180s "$base/k3s-bin" kubectl --kubeconfig "$base/kubeconfig" exec -n "$namespace" "$pod" -c workspace -- /usr/local/bin/storage-ci-probe write /storage-ci-root-fill 2415919104 > "$case_out/root-over-two-gib.json"
 jq -e '.written_bytes==2415919104 and .fsync_result==0 and .fsync_errno==0' "$case_out/root-over-two-gib.json"
 kube exec -n "$namespace" "$pod" -c workspace -- stat -c '%s %b' /storage-ci-root-fill > "$case_out/root-over-two-gib-stat.txt"
 read -r logical blocks < "$case_out/root-over-two-gib-stat.txt"
 test "$logical" = 2415919104
 test "$((blocks*512))" -ge 2415919104
 # Cross several kubelet summary/eviction cycles with the same running Pod.
 for n in $(seq 1 30); do
  check_identity
  ssh_product 'test "$(cat /home/user/storage-ci-home-marker)" = home-survives' >/dev/null
  sleep 2
 done
 volume_accounted=false
 for n in $(seq 1 60); do
  summary > "$case_out/summary-over-two-gib.json"
  if jq -e --arg uid "$uid" --arg claim "$scratch_claim" '.pods[] | select(.podRef.uid==$uid) | any(.volume[]?; .pvcRef.name==$claim and .usedBytes>=2415919104)' "$case_out/summary-over-two-gib.json" >/dev/null; then volume_accounted=true; break; fi
  check_identity
  sleep 2
 done
 test "$volume_accounted" = true
 timeout 180s "$base/k3s-bin" kubectl --kubeconfig "$base/kubeconfig" exec -n "$namespace" "$pod" -c workspace -- /usr/local/bin/storage-ci-probe fill /storage-ci-root-fill > "$case_out/root-enospc.json"
 jq -e '.write_errno==28 or .write_errno==122' "$case_out/root-enospc.json"
 kube exec -n "$namespace" "$pod" -c workspace -- /usr/local/bin/storage-ci-probe pwrite /tmp/storage-ci-temp-fill > "$case_out/tmp-enospc.json"
 kube exec -n "$namespace" "$pod" -c workspace -- /usr/local/bin/storage-ci-probe pwrite /var/lib/mwc/build-scratch/storage-ci-cache-fill > "$case_out/cache-enospc.json"
 kube exec -n "$namespace" "$pod" -c workspace -- sha256sum /etc/hosts > "$case_out/hosts-before.sha256"
 kube exec -n "$namespace" "$pod" -c workspace -- /usr/local/bin/storage-ci-probe pwrite /etc/hosts > "$case_out/hosts-enospc.json"
 jq -e '(.pwrite_errno==28 or .pwrite_errno==122) and .original_length_restored==true' "$case_out/hosts-enospc.json"
 kube exec -n "$namespace" "$pod" -c workspace -- sha256sum /etc/hosts > "$case_out/hosts-after.sha256"
 cmp "$case_out/hosts-before.sha256" "$case_out/hosts-after.sha256"
 # CRI stdout is asynchronous: assert bounded same-device logs and Pod survival,
 # without claiming that a process stdout write reports logger ENOSPC.
 kube_log_emit 1048576 MWC_LOG_FULL > "$case_out/log-full-emission.json"
 test "$(stat -Lc %d "$pod_logs")" = "$(stat -Lc %d "$publish")"
 test "$(du --summarize --block-size=1 --dereference "$pod_logs" | cut -f1)" -le 100663296
 kube exec -n "$namespace" "$pod" -c workspace -- sh -ec 'printf home-after-full > /home/user/storage-ci-after-full; test "$(cat /home/user/storage-ci-home-marker)" = home-survives'
 ssh_product 'test "$(cat /home/user/storage-ci-after-full)" = home-after-full; printf strict-ssh-full-ok' > "$case_out/strict-ssh-full.txt"
 for n in $(seq 1 15); do check_identity; sleep 2; done
 cp "$case_out/pod-current.json" "$case_out/pod-full.json"
 kube get events -n "$namespace" --field-selector "involvedObject.uid=$uid" -o json > "$case_out/pod-events.json"
 jq -e 'all(.items[]; .reason!="Evicted" and .reason!="EvictionThresholdMet")' "$case_out/pod-events.json"
 # Free fixture data before normal replacement; do not change/replace either PVC.
 kube exec -n "$namespace" "$pod" -c workspace -- sh -ec 'rm -f /storage-ci-root-fill /tmp/storage-ci-temp-fill /var/lib/mwc/build-scratch/storage-ci-cache-fill'
 kube delete pod -n "$namespace" "$pod" --wait=true --timeout=120s
 for n in $(seq 1 180); do
  new_uid="$(kube get pod -n "$namespace" "$pod" -o jsonpath='{.metadata.uid}' 2>/dev/null || true)"
  if [ -n "$new_uid" ] && [ "$new_uid" != "$uid" ]; then break; fi
  sleep 1
 done
 test -n "$new_uid"; test "$new_uid" != "$uid"
 kube wait -n "$namespace" --for=condition=Ready "pod/$pod" --timeout=300s
 test "$(kube get pvc -n "$namespace" "$scratch_claim" -o jsonpath='{.metadata.uid}')" = "$scratch_uid"
 test "$(kube get pvc -n "$namespace" "$home_claim" -o jsonpath='{.metadata.uid}')" = "$home_uid"
 kube get pod -n "$namespace" "$pod" -o json > "$case_out/pod-replacement.json"
 address="$(jq -r '.status.podIP' "$case_out/pod-replacement.json")"
 printf '[%s]:2222 %s\n' "$address" "$(cut -d ' ' -f 1,2 "$base/host-$case_id.pub")" > "$base/known-$case_id"
 kube exec -n "$namespace" "$pod" -c workspace -- cat /run/mwc-ssh/ttyd_client_key > "$base/client-$case_id"
 ssh_product 'test "$(cat /home/user/storage-ci-home-marker)" = home-survives; printf strict-ssh-new-pod-ok' > "$case_out/strict-ssh-replacement.txt"
 # Real kubelet + CSI own unpublish; no fixture umount/vol_data manipulation.
 kube delete statefulset -n "$namespace" "${pod%-0}" --wait=true --timeout=120s
 kube wait -n "$namespace" --for=delete "pod/$pod" --timeout=180s
 for n in $(seq 1 120); do
  if ! findmnt -rn -o TARGET | grep -F "/var/lib/kubelet/pods/$new_uid/" >/dev/null; then break; fi
  sleep 1
 done
 ! findmnt -rn -o TARGET | grep -F "/var/lib/kubelet/pods/$new_uid/"
 ! findmnt -rn -o TARGET | grep -F "/var/lib/kubelet/pods/$uid/"
 handles="$(jq -r '.items[].spec.csi.volumeHandle' "$case_out/actual-pvs.json")"
 kube delete pvc -n "$namespace" "$home_claim" "$scratch_claim" --wait=true --timeout=120s
 kube wait --for=delete "pv/$home_pv" "pv/$scratch_pv" --timeout=180s
 for n in $(seq 1 120); do
  remaining=0
  for handle in $handles; do if lvs --noheadings -o lv_name | tr -d ' ' | grep -Fx "$handle" >/dev/null; then remaining=1; fi; done
  if [ "$remaining" = 0 ]; then break; fi
  sleep 1
 done
 test "$remaining" = 0
 jq -n --arg runtime "$runtime" --argjson buildkit "$enabled" --arg uid "$uid" --arg new_uid "$new_uid" --arg scratch_uid "$scratch_uid" --arg home_uid "$home_uid" \
  '{accepted:true,runtime:$runtime,buildkit:$buildkit,root_written_bytes:2415919104,root_fsync:true,scratch_enospc:true,logs_rotation:true,logs_follow_reopen:true,logs_previous:true,kubelet_logs_accounted:true,kubelet_scratch_accounted:true,logs_share_scratch:true,ssh_survived:true,pod_identity_preserved_under_pressure:true,old_pod_uid:$uid,new_pod_uid:$new_uid,scratch_pvc_uid:$scratch_uid,home_pvc_uid:$home_uid,pvc_reused:true,node_unpublish_observed:true,pv_lv_deleted:true,crash_reboot_proven:false,all_templates_proven:false,eviction_settings_changed:false}' > "$case_out/acceptance.json"
done
done
snapshot
jq -es --arg renderer "$PRODUCT_RENDERER_SHA" --arg wrapper "$WRAPPER_SHA" '
 sort_by(.runtime,.buildkit) as $cases |
 if ($cases|length)==4 and ([$cases[]|[.runtime,.buildkit]]==[["mwc-native",false],["mwc-native",true],["mwc-runsc",false],["mwc-runsc",true]]) and
 all($cases[]; .accepted and .root_written_bytes>2147483648 and .root_fsync and .scratch_enospc and .logs_rotation and .logs_follow_reopen and .logs_previous and .kubelet_logs_accounted and .kubelet_scratch_accounted and .logs_share_scratch and .ssh_survived and .pod_identity_preserved_under_pressure and .pvc_reused and .node_unpublish_observed and .pv_lv_deleted)
 then {accepted:true,renderer_sha:$renderer,wrapper_sha:$wrapper,product:"Rust d507",cases:$cases,real_csi:true,production_deployed:false,full_budget_goal_complete:true}
 else error("incomplete real CSI quota runtime matrix") end' "$out/"mwc-*/acceptance.json > "$out/acceptance.json"
