#!/usr/bin/env bash
# One disposable GitHub-hosted worker. Reuse the same disks and controller in both runs.
set -euo pipefail
test "${GITHUB_ACTIONS:-}" = true
test "${RUNNER_ENVIRONMENT:-}" = github-hosted
test "$(readlink /proc/self/ns/mnt)" != "$(readlink /proc/1/ns/mnt)"
base=${1:?fixture directory}
measure=${2:-node}
case "$measure" in
  queue) driver=topolvm-node; workload=daemonset/$driver; baseline_version='0.41.1'; candidate_version='0.41.1-mwc-volume-workers' ;;
  node) driver=topolvm-node; workload=daemonset/$driver; baseline_version='0.41.1'; candidate_version='0.41.1-mwc-independent-volumes' ;;
  controller) driver=topolvm-controller; workload=deployment/$driver; baseline_version='0.41.1-mwc250ms'; candidate_version='0.41.1-mwc250ms-lv-metadata' ;;
  *) exit 2 ;;
esac
candidate_image=localhost/$driver-ci:patched
suite=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
evidence="$base/evidence"
mkdir -p "$evidence"
mount --make-shared /

k3s="$base/k3s"
export K3S_DATA_DIR="$base/k3s-data"
ns=mwc-publish-ci
node=mwc-publish-ci
image=docker.io/library/alpine@sha256:4bcff63911fcb4448bd4fdacec207030997caf25e9bea4045fa6c8c44de311d1
daemon=
watcher=
loops=()
chart_args=()
if [[ $measure == controller || $measure == queue ]]; then
  # The chart requires controller replicas to be on different hosts.
  chart_args+=(--set-string controller.updateStrategy.type=Recreate)
fi
k() { KUBECONFIG="$base/kubeconfig" "$k3s" kubectl --request-timeout=10s "$@"; }
w() { k -n "$ns" "$@"; }
cleanup() {
  code=$?
  trap - EXIT
  if [[ -n $watcher ]]; then
    kill "$watcher" 2>/dev/null || true
    wait "$watcher" 2>/dev/null || true
  fi
  if [[ -n $daemon ]]; then
    if ((code)); then
      k get pods,pvc -A -o wide > "$evidence/failing-pods.txt" 2>&1 || true
      k get events -A > "$evidence/failing-events.txt" 2>&1 || true
      k -n topolvm-system logs daemonset/topolvm-node -c topolvm-node --timestamps > "$evidence/failing-node.log" 2>&1 || true
    fi
    kill "$daemon" 2>/dev/null || true
    wait "$daemon" 2>/dev/null || true
  fi
  # Do not forcibly detach any fixture volume still mounted.
  if ((!code)); then
    vgremove -y mwc-ci-home mwc-ci-scratch > "$evidence/remove-test-vgs.log" 2>&1 || code=1
    for device in "${loops[@]}"; do losetup -d "$device" || code=1; done
  fi
  exit "$code"
}
trap cleanup EXIT

fetch() {
  curl -fsSL --retry 2 --max-time 120 -o "$base/$1" "$2"
  printf '%s  %s\n' "$3" "$base/$1" | sha256sum -c
}
fetch k3s 'https://github.com/k3s-io/k3s/releases/download/v1.36.2%2Bk3s1/k3s' 65a55ec56c24eab44383086166ec620a491952b7e23941a49ddca6e8a4c4b4de
fetch helm.tgz 'https://get.helm.sh/helm-v4.2.4-linux-amd64.tar.gz' c306b46f719b0a4da32d0f78ee21bf90ce8d602f15b22ab753f0674d1670a7f3
fetch topolvm.tgz 'https://github.com/topolvm/topolvm/releases/download/topolvm-chart-v17.2.0/topolvm-17.2.0.tgz' 4b6c070da0b38f098c9ef6b6a38ef4e2dfff52adecedc280ad92bfab408f90ee
chmod +x "$k3s"
tar -xzf "$base/helm.tgz" -C "$base"
for disk in home scratch; do
  truncate -s 6G "$base/$disk.raw"
  device=$(losetup --find --show --direct-io=on "$base/$disk.raw")
  loops+=("$device")
  pvcreate "$device"
  vgcreate "mwc-ci-$disk" "$device"
  lvcreate -L 4G --poolmetadatasize 32M -T "mwc-ci-$disk/pool"
done

"$k3s" server --data-dir "$K3S_DATA_DIR" --write-kubeconfig "$base/kubeconfig" \
  --node-name "$node" --prefer-bundled-bin --flannel-backend=host-gw --disable-network-policy \
  --disable=traefik,servicelb,metrics-server,local-storage,coredns --disable-helm-controller \
  > "$evidence/k3s.log" 2>&1 & daemon=$!
for i in {1..60}; do
  if k version --request-timeout=2s >/dev/null 2>&1; then break; fi
  kill -0 "$daemon"
  sleep 1
done
k wait --for=create "node/$node" --timeout=110s
k wait --for=condition=Ready "node/$node" --timeout=110s
"$k3s" ctr -n k8s.io images import --platform linux/amd64 "$base/../$driver-images.tar"
for i in {1..30}; do
  if "$k3s" crictl inspecti "$candidate_image" >/dev/null 2>&1; then break; fi
  sleep 1
done
"$k3s" crictl inspecti "$candidate_image" | jq -er '.status.id' > "$evidence/candidate-image.txt"
KUBECONFIG="$base/kubeconfig" "$base/linux-amd64/helm" install topolvm "$base/topolvm.tgz" \
  -n topolvm-system --create-namespace -f "$suite/../runtime-product-vm/topolvm-values.yaml" "${chart_args[@]}" --wait --timeout 5m
k create namespace "$ns"
w create serviceaccount fixture

create_fixture() {
  local name=$1 role grace=30
  if [[ $measure == queue ]]; then grace=1; fi
  local dir="$evidence/$name"
  mkdir "$dir"
  date -u +%FT%T.%NZ > "$dir/request.txt"
  for role in home scratch; do
    jq -n --arg role "$role" --arg n "$name-$role" \
      '{apiVersion:"v1",kind:"PersistentVolumeClaim",metadata:{name:$n},spec:{storageClassName:("mwc-ci-"+$role),accessModes:["ReadWriteOnce"],resources:{requests:{storage:"1Gi"}}}}' | w create -f -
  done
  jq -n --arg n "$name" --arg image "$image" --argjson grace "$grace" \
    '{apiVersion:"v1",kind:"Pod",metadata:{name:$n},spec:{serviceAccountName:"fixture",automountServiceAccountToken:false,terminationGracePeriodSeconds:$grace,
      containers:[{name:"fixture",image:$image,imagePullPolicy:"IfNotPresent",command:["sleep","3600"],
      volumeMounts:[{name:"home",mountPath:"/mnt/home"},{name:"scratch",mountPath:"/mnt/scratch"}]}],
      volumes:[{name:"home",persistentVolumeClaim:{claimName:($n+"-home")}},{name:"scratch",persistentVolumeClaim:{claimName:($n+"-scratch")}}]}}' | w create -f -
  w wait --for=condition=Ready "pod/$name" --timeout=100s
  date -u +%FT%T.%NZ > "$dir/ready.txt"
  w get pod "$name" -o json > "$dir/pod.json"
  w exec "$name" -- sh -euc 'for v in home scratch; do printf "%s" "$v" > "/mnt/$v/check"; done; sync; test "$(cat /mnt/home/check)" = home; test "$(cat /mnt/scratch/check)" = scratch'
}

inspect_fixture() {
  local name=$1 uid pvc pv handle role
  local dir="$evidence/$name"
  uid=$(jq -er .metadata.uid "$dir/pod.json")
  : > "$dir/volume-ids.tsv"
  for role in home scratch; do
    pvc=$(w get pvc "$name-$role" -o json)
    pv=$(jq -er .spec.volumeName <<< "$pvc")
    handle=$(k get pv "$pv" -o json | jq -er '.spec.csi|select(.driver=="topolvm.io").volumeHandle')
    test "$(findmnt -n -M "/var/lib/kubelet/pods/$uid/volumes/kubernetes.io~csi/$pv/mount" -o FSTYPE)" = xfs
    test "$(lvs --noheadings -o vg_name --select "lv_name=$handle" | xargs)" = "mwc-ci-$role"
    printf '%s\t%s\t%s\n' "$role" "$pv" "$handle" >> "$dir/volume-ids.tsv"
  done
  k -n topolvm-system logs daemonset/topolvm-node -c topolvm-node --timestamps | awk -v uid="$uid" 'index($0, uid) && /NodePublishVolume/' > "$dir/publish.log"
  if [[ $measure == controller || $measure == queue ]]; then
    k -n topolvm-system logs deploy/topolvm-controller -c topolvm-controller --timestamps > "$dir/create.log"
    k -n topolvm-system logs daemonset/topolvm-node -c topolvm-node --timestamps > "$dir/lv.log"
  fi
}

reclaim_fixture() {
  local name=$1 uid pv handle role i
  local dir="$evidence/$name"
  uid=$(jq -er .metadata.uid "$dir/pod.json")
  w delete pod "$name" --timeout=90s
  w delete pvc "$name-home" "$name-scratch" --timeout=90s
  while IFS=$'\t' read -r role pv handle; do
    k wait --for=delete "pv/$pv" --timeout=90s
    for i in {1..30}; do
      if ! lvs "mwc-ci-$role/$handle" >/dev/null 2>&1; then break; fi
      sleep 1
    done
    if lvs "mwc-ci-$role/$handle" >/dev/null 2>&1; then return 1; fi
    if [[ $measure == controller || $measure == queue ]]; then
      test -z "$(k get logicalvolumes.topolvm.io "$pv" --ignore-not-found -o name)"
    fi
  done < "$dir/volume-ids.tsv"
  if findmnt -rno TARGET | awk -v uid="$uid" 'index($0, uid) {found=1} END {exit !found}'; then return 1; fi
}

verify_peer() {
  local name=storage-peer expected
  expected=$(jq -cer '[.metadata.uid,.status.containerStatuses[0].containerID]' "$evidence/$name/pod.json")
  test "$(w get pod "$name" -o json | jq -cer '[.metadata.uid,.status.containerStatuses[0].containerID]')" = "$expected"
  w exec "$name" -- sh -euc 'for v in home scratch; do test "$(cat "/mnt/$v/check")" = "$v"; printf %s "$v" > "/mnt/$v/recheck"; test "$(cat "/mnt/$v/recheck")" = "$v"; done'
}

if [[ $measure == queue ]]; then
  # shellcheck source=/dev/null
  source "$suite/measure-queue.sh"
  exit 0
fi

if [[ $measure == controller ]]; then
  stock_image=localhost/topolvm-controller-ci:baseline
  "$k3s" crictl inspecti "$stock_image" | jq -er '.status.id' > "$evidence/baseline-image.txt"
  k -n topolvm-system set image "$workload" "$driver=$stock_image"
  k -n topolvm-system rollout status "$workload" --timeout=180s
  KUBECONFIG="$base/kubeconfig" "$k3s" kubectl get logicalvolumes.topolvm.io \
    --watch --output-watch-events -o json > "$evidence/logicalvolume-watch.json" & watcher=$!
else
  stock_image=$(k -n topolvm-system get "$workload" -o json | jq -er --arg driver "$driver" '.spec.template.spec.containers[]|select(.name==$driver)|.image')
fi
create_fixture storage-peer
inspect_fixture storage-peer

for variant in stock patched; do
  if [[ $variant == patched ]]; then
    k -n topolvm-system set image "$workload" "$driver=$candidate_image"
    k -n topolvm-system rollout status "$workload" --timeout=180s
    verify_peer
    version=$candidate_version
  else
    version=$baseline_version
  fi
  k -n topolvm-system exec "$workload" -c "$driver" -- "/$driver" --version > "$evidence/$variant-version.txt"
  grep -Fx "$driver version $version" "$evidence/$variant-version.txt"
  for round in warm 1 2 3; do
    name="$variant-$round"
    create_fixture "$name"
    inspect_fixture "$name"
    reclaim_fixture "$name"
  done
done
k -n topolvm-system set image "$workload" "$driver=$stock_image"
k -n topolvm-system rollout status "$workload" --timeout=180s
k -n topolvm-system exec "$workload" -c "$driver" -- "/$driver" --version | grep -Fx "$driver version $baseline_version"
verify_peer
reclaim_fixture storage-peer
printf '%s\n' '{"same_peer_pod_and_container_after_upgrade_and_rollback":true,"original_volumes_readable_writable_and_reclaimed":true}' > "$evidence/upgrade-and-rollback.json"
if [[ $measure == controller ]]; then
  kill "$watcher" 2>/dev/null || true
  wait "$watcher" 2>/dev/null || true
  watcher=
  python3 "$suite/../topolvm-controller/prepare-timings.py" "$evidence"
else
  python3 "$suite/publish-timings.py" "$evidence"
fi
