#!/usr/bin/env bash
set -euo pipefail

required() {
  local name=$1
  if [[ -z ${!name:-} ]]; then
    printf 'required environment variable is empty: %s\n' "$name" >&2
    exit 64
  fi
}

required K3S_INSTALLATION_ID
required K3S_RELEASE_NAMESPACE
required K3S_MODE
required K3S_WORKSPACE_NAMESPACE_SCOPE
command -v kubectl >/dev/null
command -v jq >/dev/null

selector="app.kubernetes.io/instance=mwc-${K3S_INSTALLATION_ID}"
owner_label='workspace.memeloop.dev/owner-installation'
workspace_label='workspace.memeloop.dev/workspace-id'
organization_label='workspace.memeloop.dev/organization-id'
user_label='workspace.memeloop.dev/owner-user-id'
managed_by_label='app.kubernetes.io/managed-by'
managed_by='memeloop-workspace-control'
resource_kinds='pod,service,deployment,statefulset,replicaset,serviceaccount,configmap,secret,pvc,networkpolicy,ingress,hpa'
if kubectl get crd httproutes.gateway.networking.k8s.io >/dev/null 2>&1; then
  resource_kinds+=',httproute'
fi
objects=$(kubectl get "$resource_kinds" \
  -n "$K3S_RELEASE_NAMESPACE" -l "$selector" -o json)
count=$(jq '.items | length' <<<"$objects")
if [[ $count == 0 ]]; then
  printf 'no installation resources matched %s in %s\n' "$selector" "$K3S_RELEASE_NAMESPACE" >&2
  exit 1
fi
mismatches=$(jq --arg key "$owner_label" --arg owner "$K3S_INSTALLATION_ID" \
  '[.items[] | select(.metadata.labels[$key] != $owner)] | length' <<<"$objects")
if [[ $mismatches != 0 ]]; then
  printf '%s installation resources are missing or have a wrong owner label\n' "$mismatches" >&2
  exit 1
fi

name="mwc-${K3S_INSTALLATION_ID}"
for cluster_resource in clusterrole clusterrolebinding; do
  actual_owner=$(kubectl get "$cluster_resource" "$name" -o json \
    | jq -r --arg key "$owner_label" '.metadata.labels[$key] // ""')
  if [[ $actual_owner != "$K3S_INSTALLATION_ID" ]]; then
    printf '%s/%s has owner %s, expected %s\n' \
      "$cluster_resource" "$name" "$actual_owner" "$K3S_INSTALLATION_ID" >&2
    exit 1
  fi
done
if [[ $K3S_MODE == sqlite ]]; then
  kubectl rollout status statefulset/"$name" -n "$K3S_RELEASE_NAMESPACE" --timeout=5m
  replicas=$(kubectl get statefulset "$name" -n "$K3S_RELEASE_NAMESPACE" -o jsonpath='{.spec.replicas}')
  [[ $replicas == 1 ]] || { printf 'SQLite StatefulSet replicas must equal 1\n' >&2; exit 1; }
  if kubectl get deployment "$name" -n "$K3S_RELEASE_NAMESPACE" >/dev/null 2>&1; then
    printf 'SQLite installation unexpectedly contains a control-plane Deployment\n' >&2
    exit 1
  fi
else
  kubectl rollout status deployment/"$name" -n "$K3S_RELEASE_NAMESPACE" --timeout=5m
  if kubectl get statefulset "$name" -n "$K3S_RELEASE_NAMESPACE" >/dev/null 2>&1; then
    printf 'PostgreSQL installation unexpectedly contains a control-plane StatefulSet\n' >&2
    exit 1
  fi
  kubectl get hpa "$name" -n "$K3S_RELEASE_NAMESPACE" >/dev/null
fi

kubectl get endpointslices.discovery.k8s.io -n "$K3S_RELEASE_NAMESPACE" \
  -l "kubernetes.io/service-name=$name" -o json \
  | jq -e '[.items[].endpoints[]? | select(.conditions.ready != false) | .addresses[]?] | length > 0' \
    >/dev/null

release_owner=$(kubectl get namespace "$K3S_RELEASE_NAMESPACE" -o json \
  | jq -r --arg key "$owner_label" '.metadata.labels[$key] // ""')
if [[ $release_owner != "$K3S_INSTALLATION_ID" ]]; then
  printf 'release namespace has owner %s, expected %s\n' \
    "$release_owner" "$K3S_INSTALLATION_ID" >&2
  exit 1
fi

workspace_namespaces=$(kubectl get namespaces -l "$owner_label=$K3S_INSTALLATION_ID" -o json)
expected_prefix="ws-${K3S_INSTALLATION_ID}-"
namespace_scope_status="$K3S_WORKSPACE_NAMESPACE_SCOPE"
case "$K3S_WORKSPACE_NAMESPACE_SCOPE" in
  dedicated)
    invalid_namespaces=$(jq --arg prefix "$expected_prefix" \
      --arg release "$K3S_RELEASE_NAMESPACE" --arg workspace "$workspace_label" \
      '[.items[] | select(.metadata.name != $release) | select(
        (.metadata.name | startswith($prefix) | not) or
        (.metadata.labels[$workspace] // "" | length == 0)
      )] | length' <<<"$workspace_namespaces")
    if [[ $invalid_namespaces != 0 ]]; then
      printf 'managed dedicated workspace namespace lacks the installation prefix or workspace ID\n' >&2
      exit 1
    fi
    ;;
  shared)
    required K3S_WORKSPACE_SHARED_NAMESPACE
    if [[ $K3S_WORKSPACE_SHARED_NAMESPACE == "$K3S_RELEASE_NAMESPACE" ]]; then
      printf 'shared workspace namespace must differ from the release namespace\n' >&2
      exit 64
    fi
    if ! shared_namespace=$(kubectl get namespace "$K3S_WORKSPACE_SHARED_NAMESPACE" \
      --ignore-not-found -o json); then
      printf 'could not inspect configured shared workspace namespace\n' >&2
      exit 1
    fi
    if [[ -z $shared_namespace ]]; then
      namespace_scope_status='shared:pending'
    else
      shared_owner=$(jq -r --arg key "$owner_label" '.metadata.labels[$key] // ""' \
        <<<"$shared_namespace")
      if [[ $shared_owner != "$K3S_INSTALLATION_ID" ]]; then
        printf 'shared workspace namespace has owner %s, expected %s\n' \
          "$shared_owner" "$K3S_INSTALLATION_ID" >&2
        exit 1
      fi
      shared_manager=$(jq -r --arg key "$managed_by_label" '.metadata.labels[$key] // ""' \
        <<<"$shared_namespace")
      if [[ $shared_manager != "$managed_by" ]]; then
        printf 'shared workspace namespace has managed-by %s, expected %s\n' \
          "$shared_manager" "$managed_by" >&2
        exit 1
      fi
      shared_workspace_labels=$(jq --arg workspace "$workspace_label" \
        --arg organization "$organization_label" --arg user "$user_label" \
        '[.metadata.labels | .[$workspace], .[$organization], .[$user] | select(. != null)] | length' \
        <<<"$shared_namespace")
      if [[ $shared_workspace_labels != 0 ]]; then
        printf 'shared workspace namespace carries workspace-scoped ownership labels\n' >&2
        exit 1
      fi
      shared_matches=$(jq --arg shared "$K3S_WORKSPACE_SHARED_NAMESPACE" \
        '[.items[] | select(.metadata.name == $shared)] | length' <<<"$workspace_namespaces")
      if [[ $shared_matches != 1 ]]; then
        printf 'shared workspace namespace is absent from installation-owned namespaces\n' >&2
        exit 1
      fi
    fi
    invalid_namespaces=$(jq --arg prefix "$expected_prefix" \
      --arg release "$K3S_RELEASE_NAMESPACE" --arg shared "$K3S_WORKSPACE_SHARED_NAMESPACE" \
      --arg workspace "$workspace_label" \
      '[.items[] | select(.metadata.name != $release and .metadata.name != $shared) | select(
        (.metadata.name | startswith($prefix) | not) or
        (.metadata.labels[$workspace] // "" | length == 0)
      )] | length' <<<"$workspace_namespaces")
    if [[ $invalid_namespaces != 0 ]]; then
      printf 'legacy dedicated namespace lacks the installation prefix or workspace ID\n' >&2
      exit 1
    fi
    ;;
  *)
    printf 'K3S_WORKSPACE_NAMESPACE_SCOPE must be dedicated or shared\n' >&2
    exit 64
    ;;
esac

mapfile -t storage_classes < <(kubectl get pvc --all-namespaces \
  -l "$owner_label=$K3S_INSTALLATION_ID" -o json \
  | jq -r '.items[].spec.storageClassName // empty' | sort -u)
for storage_class in "${storage_classes[@]}"; do
  reclaim_policy=$(kubectl get storageclass "$storage_class" -o jsonpath='{.reclaimPolicy}')
  if [[ $reclaim_policy != Delete ]]; then
    printf 'managed PVC uses StorageClass %s with reclaimPolicy=%s\n' \
      "$storage_class" "$reclaim_policy" >&2
    exit 1
  fi
done

if [[ ${K3S_EXPECT_PUBLIC_SSH:-false} != true ]]; then
  if kubectl get tcproute -n "$K3S_RELEASE_NAMESPACE" "$name-ssh" >/dev/null 2>&1; then
    printf 'an installation without public SSH unexpectedly owns a TCPRoute\n' >&2
    exit 1
  fi
else
  kubectl get tcproute -n "$K3S_RELEASE_NAMESPACE" "$name-ssh" >/dev/null
fi

if [[ ${K3S_EXPECT_PUBLIC_WEB_SHELL:-false} == true ]]; then
  required K3S_HIGRESS_NAMESPACE
  plugin_owner=$(kubectl get wasmplugin -n "$K3S_HIGRESS_NAMESPACE" "$name-web-shell-auth" -o json \
    | jq -r --arg key "$owner_label" '.metadata.labels[$key] // ""')
  if [[ $plugin_owner != "$K3S_INSTALLATION_ID" ]]; then
    printf 'Web Shell WasmPlugin has owner %s, expected %s\n' \
      "$plugin_owner" "$K3S_INSTALLATION_ID" >&2
    exit 1
  fi
fi

workspace_count=$(jq --arg release "$K3S_RELEASE_NAMESPACE" \
  '[.items[] | select(.metadata.name != $release)] | length' <<<"$workspace_namespaces")
printf 'installation verification passed: %s resources, %s managed workspace namespaces (%s)\n' \
  "$count" "$workspace_count" "$namespace_scope_status"
