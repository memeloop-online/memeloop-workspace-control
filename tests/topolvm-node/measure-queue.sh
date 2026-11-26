#!/usr/bin/env bash
# Sourced only after measure-publish.sh verifies the GitHub-hosted private mount namespace.
controller_image=localhost/topolvm-queue-ci:controller
"$k3s" crictl inspecti "$controller_image" | jq -er '.status.id' > "$evidence/queue-controller-image.txt"
k -n topolvm-system set image deploy/topolvm-controller "topolvm-controller=$controller_image"
k -n topolvm-system rollout status deploy/topolvm-controller --timeout=180s
k -n topolvm-system exec deploy/topolvm-controller -c topolvm-controller -- /topolvm-controller --version |
  grep -Fx 'topolvm-controller version 0.41.1-mwc250ms-lv-metadata'

original_command=$(k -n topolvm-system get "$workload" -o json |
  jq -cer --arg n "$driver" '.spec.template.spec.containers[]|select(.name==$n)|.command|select(.[0]=="/topolvm-node")')
switch_node() {
  local label=$1 ref=$2 version=$3 command=$4
  local patch
  patch=$(jq -cn --arg name "$driver" --arg image "$ref" --argjson command "$command" \
    '{spec:{template:{spec:{containers:[{name:$name,image:$image,command:$command}]}}}}')
  k -n topolvm-system patch "$workload" --type=strategic -p "$patch"
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

debug_command=$(jq -c '.+["--zap-log-level=debug"]' <<< "$original_command")
two_worker_command=$(jq -c '.+["--logical-volume-workers=2"]' <<< "$debug_command")
switch_node stock "$candidate_image" "$candidate_version" "$debug_command"
create_fixture storage-peer
inspect_fixture storage-peer
for variant in stock patched; do
  workers=1
  if [[ $variant == patched ]]; then
    switch_node patched "$candidate_image" "$candidate_version" "$two_worker_command"
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

# The peer retains 1 GiB in each 8 GiB virtual pool. Two 4 GiB requests fit
# individually but not together. Repeat with an explicit name and its alias.
compete_for_pool() {
  local tag=$1 vg=$2 first=$3 second=$4 suffix class i handle
  for suffix in a b; do
    class=$first
    if [[ $suffix == b ]]; then class=$second; fi
    jq -nc --arg n "queue-$tag-$suffix" --arg node "$node" --arg c "$class" \
      '{apiVersion:"topolvm.io/v1",kind:"LogicalVolume",metadata:{name:$n},
        spec:{name:$n,nodeName:$node,size:"4Gi",deviceClass:$c}}'
  done | jq -s '{apiVersion:"v1",kind:"List",items:.}' | k create -f -
  local record="$evidence/$tag-capacity.json"
  for i in {1..60}; do
    k get logicalvolume "queue-$tag-a" "queue-$tag-b" -o json > "$record"
    if jq -e 'all(.items[]; (.status.volumeID // "")!="" or .status.code==8)' "$record" >/dev/null; then break; fi
    sleep 1
  done
  handle=$(jq -er '[.items[] | select((.status.volumeID // "")!="")]|select(length==1)|.[0].status.volumeID' "$record")
  jq -e '[.items[] | select(.status.code==8 and (.status.volumeID // "")=="")]|length==1' "$record"
  lvs "$vg/$handle"
  k delete logicalvolume "queue-$tag-a" "queue-$tag-b" --timeout=90s
  if lvs "$vg/$handle" >/dev/null 2>&1; then exit 1; fi
}
compete_for_pool same-pool mwc-ci-home mwc-ci-home mwc-ci-home
compete_for_pool alias mwc-ci-scratch mwc-ci-scratch ''

switch_node rollback "$controller_image" "$baseline_version" "$original_command"
verify_peer
reclaim_fixture storage-peer
printf '%s\n' '{"same_peer_pod_and_container_after_upgrade_and_rollback":true,"original_volumes_readable_writable_and_reclaimed":true,"full_xfs_recovered_in_original_pod":true,"oversized_lv_refused":true,"same_pool_competing_capacity_serialized":true,"default_alias_capacity_serialized":true}' > "$evidence/upgrade-and-rollback.json"
python3 "$suite/queue-timings.py" "$evidence"
