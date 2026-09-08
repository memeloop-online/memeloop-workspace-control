#!/usr/bin/env bash
set -euo pipefail

: "${K3S_INSTALLATION_ID:?required}"
: "${K3S_WORKSPACE_ID:?required}"
: "${K3S_WORKSPACE_NAMESPACE:?required}"
command -v kubectl >/dev/null
command -v jq >/dev/null

canonical_namespace='memeloop-workspace-control'
if [[ $K3S_WORKSPACE_NAMESPACE != "$canonical_namespace" ]]; then
  printf 'K3S_WORKSPACE_NAMESPACE must be %s\n' "$canonical_namespace" >&2
  exit 64
fi

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

if [[ -n ${K3S_SIBLING_WORKSPACE_ID:-} && $K3S_SIBLING_WORKSPACE_ID == "$K3S_WORKSPACE_ID" ]]; then
  printf 'sibling workspace ID must differ from deleted workspace ID\n' >&2
  exit 64
fi
namespace=$(kubectl get namespace "$canonical_namespace" -o json)
namespace_owner=$(jq -r --arg key "$owner_label" '.metadata.labels[$key] // ""' \
  <<<"$namespace")
if [[ $namespace_owner != "$K3S_INSTALLATION_ID" ]]; then
  printf 'installation namespace has owner %s, expected %s\n' \
    "$namespace_owner" "$K3S_INSTALLATION_ID" >&2
  exit 1
fi
namespace_manager=$(jq -r --arg key "$managed_by_label" '.metadata.labels[$key] // ""' \
  <<<"$namespace")
if [[ $namespace_manager != "$managed_by" ]]; then
  printf 'installation namespace has managed-by %s, expected %s\n' \
    "$namespace_manager" "$managed_by" >&2
  exit 1
fi
workspace_scoped_labels=$(jq --arg workspace "$workspace_label" \
  --arg organization "$organization_label" --arg user "$user_label" \
  '[.metadata.labels | .[$workspace], .[$organization], .[$user] | select(. != null)] | length' \
  <<<"$namespace")
if [[ $workspace_scoped_labels != 0 ]]; then
  printf 'installation namespace carries workspace-scoped ownership labels\n' >&2
  exit 1
fi
if [[ -n ${K3S_SIBLING_WORKSPACE_ID:-} ]]; then
  sibling_count=$(kubectl get "$resource_kinds" -n "$canonical_namespace" \
    -l "$owner_label=$K3S_INSTALLATION_ID,$workspace_label=$K3S_SIBLING_WORKSPACE_ID" \
    -o json | jq '.items | length')
  if [[ $sibling_count == 0 ]]; then
    printf 'no sibling resources remain for workspace %s in %s\n' \
      "$K3S_SIBLING_WORKSPACE_ID" "$canonical_namespace" >&2
    exit 1
  fi
fi

printf 'workspace Kubernetes cleanup verified: %s in %s\n' \
  "$K3S_WORKSPACE_ID" "$canonical_namespace"
