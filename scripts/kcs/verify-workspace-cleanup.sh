#!/usr/bin/env bash
set -euo pipefail

: "${KCS_INSTALLATION_ID:?required}"
: "${KCS_WORKSPACE_ID:?required}"
: "${KCS_WORKSPACE_NAMESPACE:?required}"
command -v kubectl >/dev/null
command -v jq >/dev/null

expected_prefix="ws-${KCS_INSTALLATION_ID}-"
if [[ $KCS_WORKSPACE_NAMESPACE != "$expected_prefix"* ]]; then
  printf 'workspace namespace %s does not start with %s\n' \
    "$KCS_WORKSPACE_NAMESPACE" "$expected_prefix" >&2
  exit 64
fi
if kubectl get namespace "$KCS_WORKSPACE_NAMESPACE" >/dev/null 2>&1; then
  printf 'workspace namespace still exists: %s\n' "$KCS_WORKSPACE_NAMESPACE" >&2
  exit 1
fi

remaining=$(kubectl get statefulset,service,secret,configmap,pvc,networkpolicy,httproute \
  --all-namespaces -l "workspace.memeloop.dev/workspace-id=$KCS_WORKSPACE_ID" -o json 2>/dev/null \
  | jq '.items | length')
if [[ $remaining != 0 ]]; then
  printf '%s Kubernetes resources remain for workspace %s\n' "$remaining" "$KCS_WORKSPACE_ID" >&2
  exit 1
fi

printf 'workspace Kubernetes cleanup verified: %s\n' "$KCS_WORKSPACE_ID"
