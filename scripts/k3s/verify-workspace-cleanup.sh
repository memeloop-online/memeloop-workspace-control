#!/usr/bin/env bash
set -euo pipefail

: "${K3S_INSTALLATION_ID:?required}"
: "${K3S_WORKSPACE_ID:?required}"
: "${K3S_WORKSPACE_NAMESPACE:?required}"
: "${K3S_WORKSPACE_NAMESPACE_SCOPE:?required (dedicated or shared)}"
command -v kubectl >/dev/null
command -v jq >/dev/null

owner_label='workspace.memeloop.dev/owner-installation'
workspace_label='workspace.memeloop.dev/workspace-id'
organization_label='workspace.memeloop.dev/organization-id'
user_label='workspace.memeloop.dev/owner-user-id'
managed_by_label='app.kubernetes.io/managed-by'
managed_by='memeloop-workspace-control'
resource_kinds='pod,statefulset,service,serviceaccount,secret,configmap,pvc,networkpolicy,ingress'
if kubectl get crd httproutes.gateway.networking.k8s.io >/dev/null 2>&1; then
  resource_kinds+=',httproute'
fi
remaining=$(kubectl get "$resource_kinds" \
  --all-namespaces -l "$workspace_label=$K3S_WORKSPACE_ID" -o json 2>/dev/null \
  | jq '.items | length')
cluster_remaining=$(kubectl get clusterrolebinding \
  -l "$workspace_label=$K3S_WORKSPACE_ID" -o json 2>/dev/null \
  | jq '.items | length')
remaining=$((remaining + cluster_remaining))
if [[ $remaining != 0 ]]; then
  printf '%s Kubernetes resources remain for workspace %s\n' "$remaining" "$K3S_WORKSPACE_ID" >&2
  exit 1
fi

case "$K3S_WORKSPACE_NAMESPACE_SCOPE" in
  dedicated)
    expected_prefix="ws-${K3S_INSTALLATION_ID}-"
    if [[ $K3S_WORKSPACE_NAMESPACE != "$expected_prefix"* ]]; then
      printf 'dedicated workspace namespace %s does not start with %s\n' \
        "$K3S_WORKSPACE_NAMESPACE" "$expected_prefix" >&2
      exit 64
    fi
    if kubectl get namespace "$K3S_WORKSPACE_NAMESPACE" >/dev/null 2>&1; then
      printf 'dedicated workspace namespace still exists: %s\n' \
        "$K3S_WORKSPACE_NAMESPACE" >&2
      exit 1
    fi
    ;;
  shared)
    if [[ -n ${K3S_SIBLING_WORKSPACE_ID:-} && $K3S_SIBLING_WORKSPACE_ID == "$K3S_WORKSPACE_ID" ]]; then
      printf 'sibling workspace ID must differ from deleted workspace ID\n' >&2
      exit 64
    fi
    namespace=$(kubectl get namespace "$K3S_WORKSPACE_NAMESPACE" -o json)
    namespace_owner=$(jq -r --arg key "$owner_label" '.metadata.labels[$key] // ""' \
      <<<"$namespace")
    if [[ $namespace_owner != "$K3S_INSTALLATION_ID" ]]; then
      printf 'shared workspace namespace has owner %s, expected %s\n' \
        "$namespace_owner" "$K3S_INSTALLATION_ID" >&2
      exit 1
    fi
    namespace_manager=$(jq -r --arg key "$managed_by_label" '.metadata.labels[$key] // ""' \
      <<<"$namespace")
    if [[ $namespace_manager != "$managed_by" ]]; then
      printf 'shared workspace namespace has managed-by %s, expected %s\n' \
        "$namespace_manager" "$managed_by" >&2
      exit 1
    fi
    workspace_scoped_labels=$(jq --arg workspace "$workspace_label" \
      --arg organization "$organization_label" --arg user "$user_label" \
      '[.metadata.labels | .[$workspace], .[$organization], .[$user] | select(. != null)] | length' \
      <<<"$namespace")
    if [[ $workspace_scoped_labels != 0 ]]; then
      printf 'shared workspace namespace carries workspace-scoped ownership labels\n' >&2
      exit 1
    fi
    if [[ -n ${K3S_SIBLING_WORKSPACE_ID:-} ]]; then
      sibling_count=$(kubectl get "$resource_kinds" -n "$K3S_WORKSPACE_NAMESPACE" \
        -l "$owner_label=$K3S_INSTALLATION_ID,$workspace_label=$K3S_SIBLING_WORKSPACE_ID" \
        -o json | jq '.items | length')
      if [[ $sibling_count == 0 ]]; then
        printf 'no sibling resources remain for workspace %s in shared namespace %s\n' \
          "$K3S_SIBLING_WORKSPACE_ID" "$K3S_WORKSPACE_NAMESPACE" >&2
        exit 1
      fi
    fi
    ;;
  *)
    printf 'K3S_WORKSPACE_NAMESPACE_SCOPE must be dedicated or shared\n' >&2
    exit 64
    ;;
esac

printf 'workspace Kubernetes cleanup verified: %s (%s)\n' \
  "$K3S_WORKSPACE_ID" "$K3S_WORKSPACE_NAMESPACE_SCOPE"
