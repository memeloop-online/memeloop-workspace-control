#!/bin/bash
# Only an expendable GitHub-hosted VM. No synthetic CSI publication is permitted.
set -euo pipefail
: "${GITHUB_ACTIONS:?Disposable GitHub Actions VM only}"
: "${RUNNER_TEMP:?}"
: "${GITHUB_WORKSPACE:?}"
base=/var/lib/mwc-product-storage-ci
out="$GITHUB_WORKSPACE/artifacts"
fixtures="$RUNNER_TEMP/product-storage-fixtures"
namespace=memeloop-workspace-control
mkdir -p "$base" "$out" "$base/state"
chmod 0700 "$base/state"
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
 if [ -n "$k3s_pid" ]; then kill "$k3s_pid"; wait "$k3s_pid" || true; fi
 if [ -n "$containerd_pid" ]; then kill "$containerd_pid"; wait "$containerd_pid" || true; fi
 # Never detach a mounted or busy LV to turn cleanup failure into a pass.
 for vg in mwc-ci-home mwc-ci-scratch; do
  if vgs "$vg" >/dev/null 2>&1; then
   if ! vgremove -y "$vg" >> "$out/cleanup.log" 2>&1; then status=1; fi
  fi
 done
 for loop in "${loops[@]}"; do
  if ! losetup -d "$loop" >> "$out/cleanup.log" 2>&1; then status=1; fi
 done
 exit "$status"
}

# Verify exact product registry inputs before allocating any disks/cluster.
# Anonymous Harbor libraries should be pullable; a denied/missing digest is fatal.
registry_manifest() {
 local registry="$1" repo="$2" digest="$3" prefix="$4" token_url="$5" service="$6" token
 token="$(curl -fsSL --get --data-urlencode "service=$service" --data-urlencode "scope=repository:$repo:pull" "$token_url" | jq -er '.token // .access_token')"
 curl -fsSL -H "Authorization: Bearer $token" -H 'Accept: application/vnd.oci.image.index.v1+json,application/vnd.oci.image.manifest.v1+json,application/vnd.docker.distribution.manifest.v2+json' \
  "https://$registry/v2/$repo/manifests/$digest" -o "$out/$prefix-index.json"
 echo "${digest#sha256:}  $out/$prefix-index.json" | sha256sum -c -
}
registry_manifest harbor.k3s.onetwo.website library/rust-dev \
 sha256:d50798f0f8345d7295c303ec0f04b1f1e064b7d6ac9b074d87065a488957360f rust \
 https://harbor.k3s.onetwo.website/service/token harbor-registry
registry_manifest ghcr.io memeloop-online/memeloop-workspace-control-ttyd \
 sha256:cc408d2cb9c3fe457b206cffae4a15264ae15f037eac0b1f4dde86058d499012 ttyd \
 https://ghcr.io/token ghcr.io
if [ "${1:-}" = --prerequisites ]; then exit 0; fi
trap finish EXIT

curl -fsSL --retry 3 'https://github.com/k3s-io/k3s/releases/download/v1.36.2%2Bk3s1/k3s' -o "$base/k3s-bin"
echo "65a55ec56c24eab44383086166ec620a491952b7e23941a49ddca6e8a4c4b4de  $base/k3s-bin" | sha256sum -c -
chmod 0755 "$base/k3s-bin"
K3S_DATA_DIR="$base/k3s" "$base/k3s-bin" ctr version --help >/dev/null
export PATH="$base/k3s/data/current/bin:$PATH"
containerd --version | tee "$out/containerd-version.txt"
runc --version | tee "$out/runc-version.txt"
grep -F 'v2.3.2-k3s2' "$out/containerd-version.txt"
grep -Fx 'runc version 1.4.2' "$out/runc-version.txt"
uname -a > "$out/kernel.txt"
install -m 0755 "$RUNNER_TEMP/product-storage-bin/mwc-runtime-wrapper" "$base/mwc-runtime-wrapper"
for kind in home scratch; do
 if [ "$kind" = home ]; then backing=14G; pool=12G; else backing=4G; pool=3G; fi
 truncate -s "$backing" "$base/$kind.img"
 loop="$(losetup --find --show "$base/$kind.img")"; loops+=("$loop")
 pvcreate -y "$loop"
 vgcreate "mwc-ci-$kind" "$loop"
 lvcreate -L "$pool" --poolmetadatasize 32M -T "mwc-ci-$kind/pool"
done
vg_uuid="$(vgs --noheadings -o vg_uuid mwc-ci-scratch | tr -d '[:space:]-')"
pool_uuid="$(dmsetup info -c --noheadings -o uuid mwc--ci--scratch-pool-tpool | xargs)"
jq -n --arg runtime "$(readlink -f "$(command -v runc)")" --arg vg "$vg_uuid" --arg pool "$pool_uuid" --arg dmsetup "$(readlink -f "$(command -v dmsetup)")" --arg base "$base" \
 '{real_runtime:$runtime,kubelet_root:"/var/lib/kubelet",cri_root:($base+"/containerd-root/io.containerd.grpc.v1.cri"),workspace_namespace:"memeloop-workspace-control",pod_log_root:"/var/log/pods",bundle_root:($base+"/containerd-state/io.containerd.runtime.v2.task/k8s.io"),snapshot_root:($base+"/containerd-root/io.containerd.snapshotter.v1.overlayfs/snapshots"),state_root:($base+"/state"),mount_namespace:"/proc/1/ns/mnt",scratch_vg_uuid:$vg,scratch_pool_dm_uuid:$pool,dmsetup:$dmsetup}' > /etc/mwc-runtime-wrapper.json
chmod 0600 /etc/mwc-runtime-wrapper.json
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
  [plugins.'io.containerd.cri.v1.runtime'.containerd.runtimes.mwc-storage-ci]
    runtime_type = 'io.containerd.runc.v2'
    [plugins.'io.containerd.cri.v1.runtime'.containerd.runtimes.mwc-storage-ci.options]
      BinaryName = '$base/mwc-runtime-wrapper'
      SystemdCgroup = true
TOML
containerd --config "$base/containerd.toml" > "$out/containerd.log" 2>&1 & containerd_pid=$!
for n in $(seq 1 60); do if cri info > "$out/cri-info.json" 2>/dev/null; then break; fi; sleep 1; done
cri info > "$out/cri-info.json"
# Keep actual production limits and security contexts. Ubuntu's VM policy must
# permit the existing unprivileged RootlessKit user namespace, as main CI does.
sysctl -w kernel.apparmor_restrict_unprivileged_userns=0
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
metadata: {name: mwc-storage-ci}
handler: mwc-storage-ci
YAML
curl -fsSL https://get.helm.sh/helm-v4.2.4-linux-amd64.tar.gz -o "$base/helm.tgz"
echo "c306b46f719b0a4da32d0f78ee21bf90ce8d602f15b22ab753f0674d1670a7f3  $base/helm.tgz" | sha256sum -c -
tar -xzf "$base/helm.tgz" -C "$base"
curl -fsSL https://github.com/topolvm/topolvm/releases/download/topolvm-chart-v17.2.0/topolvm-17.2.0.tgz -o "$base/topolvm.tgz"
echo "4b6c070da0b38f098c9ef6b6a38ef4e2dfff52adecedc280ad92bfab408f90ee  $base/topolvm.tgz" | sha256sum -c -
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

for enabled in false true; do
 fixture="$fixtures/buildkit-$enabled.json"
 case_out="$out/buildkit-$enabled"; mkdir -p "$case_out"
 pod="$(jq -r '.names.pod' "$fixture")"
 home_claim="$(jq -r '.names.home' "$fixture")"
 scratch_claim="$(jq -r '.names.scratch' "$fixture")"
 # These are precisely the ResourceBuilder's real config/projections/services.
 jq '{apiVersion:"v1",kind:"List",items:([.namespace,.scratch,.service,.config]+.injections)}' "$fixture" | kube apply -f -
 identity="$(jq -r '.identity.metadata.name' "$fixture")"
 ssh-keygen -q -t ed25519 -N '' -f "$base/host-$enabled"
 kube create secret generic -n "$namespace" "$identity" \
  --from-file="ssh_host_ed25519_key=$base/host-$enabled" --from-file="ssh_host_ed25519_key.pub=$base/host-$enabled.pub"
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
 kube exec -n "$namespace" "$pod" -c workspace -- cat /run/mwc-ssh/ttyd_client_key > "$base/client-$enabled"
 chmod 0600 "$base/client-$enabled"
 address="$(jq -r '.status.podIP' "$case_out/pod-initial.json")"
 printf '[%s]:2222 %s\n' "$address" "$(cut -d ' ' -f 1,2 "$base/host-$enabled.pub")" > "$base/known-$enabled"
 ssh_product() {
  ssh -F /dev/null -p 2222 -o BatchMode=yes -o StrictHostKeyChecking=yes -o ConnectTimeout=5 \
   -o "UserKnownHostsFile=$base/known-$enabled" -i "$base/client-$enabled" "user@$address" "$@"
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
 jq -s --arg uid "$uid" --arg publish "$publish" '[.[]|select(.pod_uid==$uid)] | select(length>=3) | if all(.[];.scratch.path==$publish and (.upper|startswith($publish+"/runtime-private/"))) then . else error("incorrect upper budget") end' "$base/state/"*.json > "$case_out/actual-wrapper-records.json"
 kube exec -i -n "$namespace" "$pod" -c workspace -- sh -ec 'cat > /usr/local/bin/storage-ci-probe; chmod 0755 /usr/local/bin/storage-ci-probe' < "$RUNNER_TEMP/product-storage-bin/mwc-budget-probe"
 kube exec -n "$namespace" "$pod" -c workspace -- /usr/local/bin/storage-ci-probe fill /storage-ci-root-fill > "$case_out/root-enospc.json"
 kube exec -n "$namespace" "$pod" -c workspace -- /usr/local/bin/storage-ci-probe pwrite /tmp/storage-ci-temp-fill > "$case_out/tmp-enospc.json"
 kube exec -n "$namespace" "$pod" -c workspace -- /usr/local/bin/storage-ci-probe pwrite /var/lib/mwc/build-scratch/storage-ci-cache-fill > "$case_out/cache-enospc.json"
 kube exec -n "$namespace" "$pod" -c workspace -- /usr/local/bin/storage-ci-probe pwrite /etc/hosts > "$case_out/hosts-enospc.json"
 # stdout's CRI logger is asynchronous; its fd writes are not claimed to return
 # ENOSPC to the process. Logs share the verified filesystem and remain bounded.
 test "$(stat -Lc %d "/var/log/pods/${namespace}_${pod}_${uid}")" = "$(stat -Lc %d "$publish")"
 findmnt -J -T "/var/log/pods/${namespace}_${pod}_${uid}" > "$case_out/logs-shared-mount.json"
 kube exec -n "$namespace" "$pod" -c workspace -- sh -ec 'printf home-after-full > /home/user/storage-ci-after-full; test "$(cat /home/user/storage-ci-home-marker)" = home-survives'
 ssh_product 'test "$(cat /home/user/storage-ci-after-full)" = home-after-full; printf strict-ssh-full-ok' > "$case_out/strict-ssh-full.txt"
 kube get pod -n "$namespace" "$pod" -o json > "$case_out/pod-full.json"
 jq -e '.status.phase=="Running" and all(.status.containerStatuses[];.ready)' "$case_out/pod-full.json"
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
 printf '[%s]:2222 %s\n' "$address" "$(cut -d ' ' -f 1,2 "$base/host-$enabled.pub")" > "$base/known-$enabled"
 kube exec -n "$namespace" "$pod" -c workspace -- cat /run/mwc-ssh/ttyd_client_key > "$base/client-$enabled"
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
 jq -n --arg uid "$uid" --arg new_uid "$new_uid" --arg scratch_uid "$scratch_uid" --arg home_uid "$home_uid" \
  '{accepted:true,scope:"actual Rust image + ttyd + optional BuildKit, normal TopoLVM CSI lifecycle only",old_pod_uid:$uid,new_pod_uid:$new_uid,scratch_pvc_uid:$scratch_uid,home_pvc_uid:$home_uid,pvc_reused:true,node_unpublish_observed:true,pv_lv_deleted:true,crash_reboot_proven:false,all_templates_proven:false,eviction_settings_changed:false}' > "$case_out/acceptance.json"
done
snapshot
jq -n --arg renderer "$PRODUCT_RENDERER_SHA" --arg wrapper "$WRAPPER_SHA" \
 '{accepted:true,renderer_sha:$renderer,wrapper_sha:$wrapper,product:"Rust d507",buildkit_cases:[false,true],real_csi:true,production_deployed:false,full_budget_goal_complete:false}' > "$out/acceptance.json"
