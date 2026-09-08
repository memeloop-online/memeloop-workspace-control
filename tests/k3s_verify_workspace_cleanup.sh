#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
verifier="$repo_root/scripts/k3s/verify-workspace-cleanup.sh"
target_id='00000000-0000-0000-0000-000000000001'
sibling_id='00000000-0000-0000-0000-000000000002'
resource_kinds='pod,statefulset,service,serviceaccount,secret,configmap,pvc,networkpolicy,ingress'

kubectl() {
  if [[ $1 == get && $2 == crd ]]; then
    return 1
  fi
  if [[ $1 == get && $2 == clusterrolebinding ]]; then
    if [[ ${FAKE_TARGET_CLUSTER_RESOURCE:-false} == true ]]; then
      printf '{"items":[{}]}'
    else
      printf '{"items":[]}'
    fi
    return
  fi
  if [[ $1 == get && $2 == "$resource_kinds" ]]; then
    if [[ " $* " == *" --all-namespaces "* ]]; then
      if [[ ${FAKE_TARGET_RESOURCE:-false} == true ]]; then
        printf '{"items":[{}]}'
      else
        printf '{"items":[]}'
      fi
    elif [[ ${FAKE_SIBLING_RESOURCE:-true} == true ]]; then
      printf '{"items":[{}]}'
    else
      printf '{"items":[]}'
    fi
    return
  fi
  if [[ $1 == get && $2 == namespace ]]; then
    if [[ " $* " != *" -o json " ]]; then
      [[ ${FAKE_NAMESPACE_EXISTS:-false} == true ]]
      return
    fi
    local workspace_label=''
    if [[ ${FAKE_SHARED_WORKSPACE_LABEL:-false} == true ]]; then
      workspace_label=',"workspace.memeloop.dev/workspace-id":"unexpected"'
    fi
    printf '{"metadata":{"labels":{"workspace.memeloop.dev/owner-installation":"%s","app.kubernetes.io/managed-by":"%s"%s}}}' \
      "${FAKE_SHARED_OWNER:-test-a}" "${FAKE_SHARED_MANAGER:-memeloop-workspace-control}" \
      "$workspace_label"
    return
  fi
  printf 'unexpected kubectl arguments: %q' "$@" >&2
  printf '\n' >&2
  return 2
}
export -f kubectl
export resource_kinds

K3S_INSTALLATION_ID=test-a \
K3S_WORKSPACE_ID=$target_id \
K3S_SIBLING_WORKSPACE_ID=$sibling_id \
K3S_WORKSPACE_NAMESPACE=memeloop-workspace-control \
bash "$verifier" >/dev/null

K3S_INSTALLATION_ID=test-a \
K3S_WORKSPACE_ID=$target_id \
K3S_WORKSPACE_NAMESPACE=memeloop-workspace-control \
bash "$verifier" >/dev/null

if FAKE_TARGET_RESOURCE=true \
  K3S_INSTALLATION_ID=test-a \
  K3S_WORKSPACE_ID=$target_id \
  K3S_SIBLING_WORKSPACE_ID=$sibling_id \
  K3S_WORKSPACE_NAMESPACE=memeloop-workspace-control \
  bash "$verifier" >/dev/null 2>&1; then
  printf 'cleanup accepted a remaining target workspace resource\n' >&2
  exit 1
fi

if FAKE_TARGET_CLUSTER_RESOURCE=true \
  K3S_INSTALLATION_ID=test-a \
  K3S_WORKSPACE_ID=$target_id \
  K3S_WORKSPACE_NAMESPACE=memeloop-workspace-control \
  bash "$verifier" >/dev/null 2>&1; then
  printf 'cleanup accepted a remaining target ClusterRoleBinding\n' >&2
  exit 1
fi

if FAKE_SHARED_OWNER=other-installation \
  K3S_INSTALLATION_ID=test-a \
  K3S_WORKSPACE_ID=$target_id \
  K3S_SIBLING_WORKSPACE_ID=$sibling_id \
  K3S_WORKSPACE_NAMESPACE=memeloop-workspace-control \
  bash "$verifier" >/dev/null 2>&1; then
  printf 'cleanup accepted a namespace owned by another installation\n' >&2
  exit 1
fi

if FAKE_SHARED_WORKSPACE_LABEL=true \
  K3S_INSTALLATION_ID=test-a \
  K3S_WORKSPACE_ID=$target_id \
  K3S_SIBLING_WORKSPACE_ID=$sibling_id \
  K3S_WORKSPACE_NAMESPACE=memeloop-workspace-control \
  bash "$verifier" >/dev/null 2>&1; then
  printf 'cleanup accepted workspace ownership on the installation namespace\n' >&2
  exit 1
fi

if FAKE_SIBLING_RESOURCE=false \
  K3S_INSTALLATION_ID=test-a \
  K3S_WORKSPACE_ID=$target_id \
  K3S_SIBLING_WORKSPACE_ID=$sibling_id \
  K3S_WORKSPACE_NAMESPACE=memeloop-workspace-control \
  bash "$verifier" >/dev/null 2>&1; then
  printf 'cleanup accepted disappearance of sibling resources\n' >&2
  exit 1
fi

if K3S_INSTALLATION_ID=test-a \
  K3S_WORKSPACE_ID=$target_id \
  K3S_WORKSPACE_NAMESPACE=another-namespace \
  bash "$verifier" >/dev/null 2>&1; then
  printf 'cleanup verification accepted a non-canonical namespace\n' >&2
  exit 1
fi

printf 'K3s workspace cleanup verification tests passed\n'
