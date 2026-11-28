#!/usr/bin/env bash
# Sourced only in the GitHub-hosted fixture after its private mounts are ready.
switch_lvmd() {
  local arm=$1 version=$2 ref="localhost/lvmd-ci:$1" image_ids
  image_ids=$("$k3s" crictl inspecti "$ref" |
    jq -cer '.status | [.id, ((.repoDigests // [])[] | split("@")[-1])]')
  k -n topolvm-system set image "$workload" "lvmd=$ref"
  k -n topolvm-system rollout status "$workload" --timeout=180s
  k -n topolvm-system exec "$workload" -c lvmd -- /lvmd --version |
    grep -Fx "lvmd version $version" > "$evidence/lvmd-$arm-version.txt"
  local pod
  pod=$(k -n topolvm-system get pod -l app.kubernetes.io/component=lvmd,idx=0 -o json)
  jq -e --argjson images "$image_ids" '
    [.items[] | select(.metadata.deletionTimestamp==null) | .status.containerStatuses[]
      | select(.name=="lvmd" and .ready) | .imageID | split("@")[-1] | split("://")[-1]]
    | length==1 and (.[0] as $id | $images | index($id)!=null)
  ' <<< "$pod"
}

switch_lvmd baseline "$baseline_version"
create_fixture storage-peer
inspect_fixture storage-peer
for arm in baseline patched; do
  if [[ $arm == patched ]]; then
    switch_lvmd patched "$candidate_version"
    verify_peer
  fi
  for trial in warm 1 2 3; do
    name="lvmd-$arm-$trial"
    create_fixture "$name"
    inspect_fixture "$name"
    reclaim_fixture "$name"
  done
done

# Expand the original Home volume through CSI while the candidate serves it.
w patch pvc storage-peer-home --type merge -p '{"spec":{"resources":{"requests":{"storage":"2Gi"}}}}'
w wait --for=jsonpath='{.status.capacity.storage}'=2Gi pvc/storage-peer-home --timeout=110s
w exec storage-peer -- sh -ec 'test "$(df -Pk /mnt/home | awk "NR==2 {print \$2}")" -ge 1500000'
verify_peer
k -n topolvm-system logs "$workload" -c lvmd --timestamps > "$evidence/lvmd-patched.log"

switch_lvmd baseline "$baseline_version"
verify_peer
reclaim_fixture storage-peer
printf '%s\n' '{"both_mounts_created_writable_and_reclaimed":true,"peer_home_expanded_online":true,"same_peer_pod_and_container_after_upgrade_and_rollback":true}' > "$evidence/lvmd-upgrade-and-rollback.json"
