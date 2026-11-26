#!/usr/bin/env bash
# Sourced only after measure-publish.sh verifies the GitHub-hosted private mount namespace.
stock_image=localhost/topolvm-node-ci:baseline
controller_image=localhost/topolvm-queue-ci:controller
for ref in "$stock_image" "$controller_image"; do
  "$k3s" crictl inspecti "$ref" | jq -er '.status.id'
done > "$evidence/queue-input-images.txt"
k -n topolvm-system set image deploy/topolvm-controller "topolvm-controller=$controller_image"
k -n topolvm-system rollout status deploy/topolvm-controller --timeout=180s
k -n topolvm-system exec deploy/topolvm-controller -c topolvm-controller -- /topolvm-controller --version |
  grep -Fx 'topolvm-controller version 0.41.1-mwc250ms-lv-metadata'

switch_node() {
  local label=$1 ref=$2 version=$3
  k -n topolvm-system set image "$workload" "$driver=$ref"
  k -n topolvm-system rollout status "$workload" --timeout=180s
  k -n topolvm-system exec "$workload" -c "$driver" -- "/$driver" --version |
    grep -Fx "$driver version $version" > "$evidence/$label-version.txt"
}
queue_metrics() {
  local label=$1 expected=$2 pod
  pod=$(k -n topolvm-system get "$workload" -o json | jq -er '.spec.selector.matchLabels|to_entries|map(.key+"="+.value)|join(",")')
  pod=$(k -n topolvm-system get pod -l "$pod" --field-selector=status.phase=Running -o json | jq -er '.items | select(length==1) | .[0].metadata.name')
  k get --raw "/api/v1/namespaces/topolvm-system/pods/$pod:8080/proxy/metrics" |
    awk '/^controller_runtime_max_concurrent_reconciles|^process_start_time_seconds|^workqueue_(queue_duration_seconds|work_duration_seconds)/ && (/logicalvolume/ || /^process_start_time_seconds/) {print}' > "$evidence/$label.prom"
  grep -Fx "controller_runtime_max_concurrent_reconciles{controller=\"logicalvolume\"} $expected" "$evidence/$label.prom"
}
test_group() {
  local variant=$1 trial=$2 count=$3 i pid status=0
  local -a names=() tids=()
  for ((i=1; i<=count; i++)); do
    names+=("$variant-$trial-$i")
    create_fixture "${names[-1]}" > "$evidence/${names[-1]}.log" 2>&1 &
    tids+=("$!")
  done
  for pid in "${tids[@]}"; do wait "$pid" || status=1; done
  ((status == 0))
  for name in "${names[@]}"; do inspect_fixture "$name"; done
  for name in "${names[@]}"; do reclaim_fixture "$name"; done
  verify_peer
}

switch_node stock "$stock_image" "$baseline_version"
create_fixture storage-peer
inspect_fixture storage-peer
for variant in stock patched; do
  workers=1
  if [[ $variant == patched ]]; then
    switch_node patched "$candidate_image" "$candidate_version"
    verify_peer
    workers=2
  fi
  test_group "$variant" warm 1
  queue_metrics "$variant-before" "$workers"
  for trial in 1 2 3; do test_group "$variant" "serial$trial" 1; done
  for trial in 1 2; do test_group "$variant" "parallel$trial" 4; done
  queue_metrics "$variant-after" "$workers"
done

# Exercise real ENOSPC on one XFS mount. The sibling stays writable, and the
# original Pod and both files survive native online growth of the full mount.
name=patched-growth
create_fixture "$name"
inspect_fixture "$name"
before_uid=$(jq -er '.metadata.uid' "$evidence/$name/pod.json")
before_containers=$(jq -cer '[.status.containerStatuses[].containerID]' "$evidence/$name/pod.json")
w exec "$name" -- sh -euc '
  if dd if=/dev/zero of=/mnt/home/full bs=1M count=1500 conv=fsync 2> /tmp/dd-error; then exit 1; fi
  grep -F "No space left on device" /tmp/dd-error
  printf %s scratch > /mnt/scratch/full-sibling
  test "$(cat /mnt/home/check)" = home
  test "$(cat /mnt/scratch/full-sibling)" = scratch
' > "$evidence/full-xfs.log"
w patch pvc "$name-home" --type merge -p '{"spec":{"resources":{"requests":{"storage":"2Gi"}}}}'
for i in {1..90}; do
  mounted_size=$(w exec "$name" -- df -kP /mnt/home | awk 'NR==2 {print $2}')
  if ((mounted_size > 1500000)); then break; fi
  sleep 1
done
((mounted_size > 1500000))
w get pod "$name" -o json | jq -es --arg uid "$before_uid" --argjson ids "$before_containers"   '.[0].metadata.uid==$uid and [.[0].status.containerStatuses[].containerID]==$ids'
w exec "$name" -- sh -euc '
  test "$(cat /mnt/home/check)" = home
  test "$(cat /mnt/scratch/full-sibling)" = scratch
  printf %s recovered > /mnt/home/after
  sync
  test "$(cat /mnt/home/after)" = recovered
'
reclaim_fixture "$name"

# Send an impossible native create request to the node so the scheduler
# cannot mask a missing pool check. This records rejection without filling a VG.
k api-resources -o name | grep -Fx logicalvolumes.topolvm.io
cat <<JSON | k create -f -
{"apiVersion":"topolvm.io/v1","kind":"LogicalVolume","metadata":{"name":"queue-too-large"},"spec":{"name":"queue-too-large","nodeName":"$node","size":"100Gi","deviceClass":"mwc-ci-home"}}
JSON
k wait --for=jsonpath='{.status.code}'=8 logicalvolume/queue-too-large --timeout=60s
k get logicalvolume queue-too-large -o json > "$evidence/capacity-refusal.json"
jq -e '.status.code==8 and (.status.volumeID // "")==""' "$evidence/capacity-refusal.json"
k delete logicalvolume queue-too-large --timeout=90s

# The peer retains 1 GiB in an 8 GiB virtual Home pool. Two distinct 4 GiB
# requests fit individually but not together. Start both through the node.
for suffix in a b; do
  jq -nc --arg n "queue-capacity-$suffix" --arg node "$node" \
    '{apiVersion:"topolvm.io/v1",kind:"LogicalVolume",metadata:{name:$n},
      spec:{name:$n,nodeName:$node,size:"4Gi",deviceClass:"mwc-ci-home"}}'
done | jq -s '{apiVersion:"v1",kind:"List",items:.}' | k create -f -
for i in {1..60}; do
  k get logicalvolume queue-capacity-a queue-capacity-b -o json > "$evidence/same-pool-capacity.json"
  if jq -e 'all(.items[]; (.status.volumeID // "")!="" or .status.code==8)' "$evidence/same-pool-capacity.json" >/dev/null; then break; fi
  sleep 1
done
handle=$(jq -er '[.items[]|select((.status.volumeID // "")!="")]|select(length==1)|.[0].status.volumeID' "$evidence/same-pool-capacity.json")
jq -e '[.items[]|select(.status.code==8 and (.status.volumeID // "")=="")]|length==1' "$evidence/same-pool-capacity.json"
lvs "mwc-ci-home/$handle"
k delete logicalvolume queue-capacity-a queue-capacity-b --timeout=90s
if lvs "mwc-ci-home/$handle" >/dev/null 2>&1; then exit 1; fi

switch_node rollback "$stock_image" "$baseline_version"
verify_peer
reclaim_fixture storage-peer
printf '%s\n' '{"same_peer_pod_and_container_after_upgrade_and_rollback":true,"original_volumes_readable_writable_and_reclaimed":true,"full_xfs_recovered_in_original_pod":true,"oversized_lv_refused":true,"same_pool_competing_capacity_serialized":true}' > "$evidence/upgrade-and-rollback.json"
python3 "$suite/queue-timings.py" "$evidence"
