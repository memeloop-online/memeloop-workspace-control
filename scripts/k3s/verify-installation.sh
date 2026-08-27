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
command -v kubectl >/dev/null
command -v jq >/dev/null

selector="app.kubernetes.io/instance=mwc-${K3S_INSTALLATION_ID}"
owner_label='workspace.memeloop.dev/owner-installation'
resource_kinds='pod,service,deployment,statefulset,replicaset,serviceaccount,configmap,secret,pvc,networkpolicy,hpa'
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

kubectl get endpoints "$name" -n "$K3S_RELEASE_NAMESPACE" -o json \
  | jq -e '[.subsets[]?.addresses[]?] | length > 0' >/dev/null

workspace_namespaces=$(kubectl get namespaces -l "$owner_label=$K3S_INSTALLATION_ID" -o json)
bad_prefix=$(jq --arg prefix "ws-${K3S_INSTALLATION_ID}-" \
  '[.items[].metadata.name | select(startswith($prefix) | not)] | length' <<<"$workspace_namespaces")
if [[ $bad_prefix != 0 ]]; then
  printf 'managed workspace namespace does not use the installation prefix\n' >&2
  exit 1
fi

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

printf 'installation verification passed: %s resources, %s managed workspace namespaces\n' \
  "$count" "$(jq '.items | length' <<<"$workspace_namespaces")"
