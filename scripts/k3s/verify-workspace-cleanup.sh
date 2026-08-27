#!/usr/bin/env bash
set -euo pipefail

: "${K3S_INSTALLATION_ID:?required}"
: "${K3S_WORKSPACE_ID:?required}"
: "${K3S_WORKSPACE_NAMESPACE:?required}"
command -v kubectl >/dev/null
command -v jq >/dev/null

expected_prefix="ws-${K3S_INSTALLATION_ID}-"
if [[ $K3S_WORKSPACE_NAMESPACE != "$expected_prefix"* ]]; then
  printf 'workspace namespace %s does not start with %s\n' \
    "$K3S_WORKSPACE_NAMESPACE" "$expected_prefix" >&2
  exit 64
fi
if kubectl get namespace "$K3S_WORKSPACE_NAMESPACE" >/dev/null 2>&1; then
  printf 'workspace namespace still exists: %s\n' "$K3S_WORKSPACE_NAMESPACE" >&2
  exit 1
fi

resource_kinds='statefulset,service,secret,configmap,pvc,networkpolicy,ingress'
if kubectl get crd httproutes.gateway.networking.k8s.io >/dev/null 2>&1; then
  resource_kinds+=',httproute'
fi
remaining=$(kubectl get "$resource_kinds" \
  --all-namespaces -l "workspace.memeloop.dev/workspace-id=$K3S_WORKSPACE_ID" -o json 2>/dev/null \
  | jq '.items | length')
if [[ $remaining != 0 ]]; then
  printf '%s Kubernetes resources remain for workspace %s\n' "$remaining" "$K3S_WORKSPACE_ID" >&2
  exit 1
fi

printf 'workspace Kubernetes cleanup verified: %s\n' "$K3S_WORKSPACE_ID"
