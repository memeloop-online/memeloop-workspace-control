#!/usr/bin/env bash
# One disposable GitHub-hosted worker. Reuse the same disks and controller in both runs.
set -euo pipefail
test "${GITHUB_ACTIONS:-}" = true
test "${RUNNER_ENVIRONMENT:-}" = github-hosted
test "$(readlink /proc/self/ns/mnt)" != "$(readlink /proc/1/ns/mnt)"
base=${1:?fixture directory}
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
loops=()
k() { KUBECONFIG="$base/kubeconfig" "$k3s" kubectl --request-timeout=10s "$@"; }
w() { k -n "$ns" "$@"; }
cleanup() {
  code=$?
  trap - EXIT
  if [[ -n $daemon ]]; then
    if ((code)); then
      k get pods,pvc -A -o wide > "$evidence/failing-pods.txt" 2>&1 || true
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
"$k3s" ctr -n k8s.io images import --platform linux/amd64 "$base/../topolvm-node-images.tar"
for i in {1..30}; do
  if "$k3s" crictl inspecti localhost/topolvm-node-ci:patched >/dev/null 2>&1; then break; fi
  sleep 1
done
"$k3s" crictl inspecti localhost/topolvm-node-ci:patched | jq -er '.status.id' > "$evidence/candidate-image.txt"
KUBECONFIG="$base/kubeconfig" "$base/linux-amd64/helm" install topolvm "$base/topolvm.tgz" \
  -n topolvm-system --create-namespace -f "$suite/../runtime-product-vm/topolvm-values.yaml" --wait --timeout 5m
k create namespace "$ns"
w create serviceaccount fixture

sample() {
  local name=$1 uid pvc pv handle role i
  local dir="$evidence/$name"
  mkdir "$dir"
  date -u +%FT%T.%NZ > "$dir/request.txt"
  for role in home scratch; do
    jq -n --arg role "$role" --arg n "$name-$role" \
      '{apiVersion:"v1",kind:"PersistentVolumeClaim",metadata:{name:$n},spec:{storageClassName:("mwc-ci-"+$role),accessModes:["ReadWriteOnce"],resources:{requests:{storage:"1Gi"}}}}' | w create -f -
  done
  jq -n --arg n "$name" --arg image "$image" \
    '{apiVersion:"v1",kind:"Pod",metadata:{name:$n},spec:{serviceAccountName:"fixture",automountServiceAccountToken:false,
      containers:[{name:"fixture",image:$image,imagePullPolicy:"IfNotPresent",command:["sleep","3600"],
      volumeMounts:[{name:"home",mountPath:"/mnt/home"},{name:"scratch",mountPath:"/mnt/scratch"}]}],
      volumes:[{name:"home",persistentVolumeClaim:{claimName:($n+"-home")}},{name:"scratch",persistentVolumeClaim:{claimName:($n+"-scratch")}}]}}' | w create -f -
  w wait --for=condition=Ready "pod/$name" --timeout=100s
  date -u +%FT%T.%NZ > "$dir/ready.txt"
  w get pod "$name" -o json > "$dir/pod.json"
  uid=$(jq -er .metadata.uid "$dir/pod.json")
  w exec "$name" -- sh -euc 'for v in home scratch; do printf "%s" "$v" > "/mnt/$v/check"; done; sync; test "$(cat /mnt/home/check)" = home; test "$(cat /mnt/scratch/check)" = scratch'
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
  w delete pod "$name" --timeout=90s
  w delete pvc "$name-home" "$name-scratch" --timeout=90s
  while IFS=$'\t' read -r role pv handle; do
    k wait --for=delete "pv/$pv" --timeout=90s
    for i in {1..30}; do
      if ! lvs "mwc-ci-$role/$handle" >/dev/null 2>&1; then break; fi
      sleep 1
    done
    if lvs "mwc-ci-$role/$handle" >/dev/null 2>&1; then return 1; fi
  done < "$dir/volume-ids.tsv"
  if findmnt -rno TARGET | awk -v uid="$uid" 'index($0, uid) {found=1} END {exit !found}'; then return 1; fi
}

for variant in stock patched; do
  if [[ $variant == patched ]]; then
    k -n topolvm-system set image daemonset/topolvm-node topolvm-node=localhost/topolvm-node-ci:patched
    k -n topolvm-system rollout status daemonset/topolvm-node --timeout=180s
  fi
  k -n topolvm-system exec daemonset/topolvm-node -c topolvm-node -- /topolvm-node --version > "$evidence/$variant-version.txt"
  if [[ $variant == patched ]]; then
    grep -F '0.41.1-mwc-independent-volumes' "$evidence/$variant-version.txt"
  else
    grep -E ' 0\.41\.1$' "$evidence/$variant-version.txt"
  fi
  for round in warm 1 2 3; do sample "$variant-$round"; done
done
python3 "$suite/publish-timings.py" "$evidence"
