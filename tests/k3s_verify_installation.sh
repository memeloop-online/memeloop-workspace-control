#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
verifier="$repo_root/scripts/k3s/verify-installation.sh"
owner_label='workspace.memeloop.dev/owner-installation'
workspace_label='workspace.memeloop.dev/workspace-id'

kubectl() {
  if [[ $1 == get && $2 == crd ]]; then
    return 1
  fi
  if [[ $1 == get && $2 == pod,service,deployment,statefulset,replicaset,serviceaccount,configmap,secret,pvc,networkpolicy,ingress,hpa ]]; then
    printf '{"items":[{"metadata":{"labels":{"app.kubernetes.io/instance":"mwc-test-a","%s":"test-a"}}}]}' "$owner_label"
    return
  fi
  if [[ $1 == get && ( $2 == clusterrole || $2 == clusterrolebinding ) ]]; then
    printf '{"metadata":{"labels":{"%s":"test-a"}}}' "$owner_label"
    return
  fi
  if [[ $1 == rollout && $2 == status ]]; then
    return
  fi
  if [[ $1 == get && $2 == statefulset ]]; then
    printf '1'
    return
  fi
  if [[ $1 == get && $2 == deployment ]]; then
    return 1
  fi
  if [[ $1 == get && $2 == endpointslices.discovery.k8s.io ]]; then
    printf '{"items":[{"endpoints":[{"conditions":{"ready":true},"addresses":["10.42.0.10"]}]}]}'
    return
  fi
  if [[ $1 == get && $2 == namespace ]]; then
    if [[ $3 == workspace-shared ]]; then
      if [[ ${FAKE_SHARED_ABSENT:-false} == true ]]; then
        return
      fi
      local workspace_fragment=''
      if [[ ${FAKE_SHARED_WORKSPACE_LABEL:-false} == true ]]; then
        workspace_fragment=',"%s":"unexpected"'
      fi
      if [[ -n $workspace_fragment ]]; then
        printf '{"metadata":{"labels":{"%s":"%s","app.kubernetes.io/managed-by":"%s","%s":"unexpected"}}}' \
          "$owner_label" "${FAKE_SHARED_OWNER:-test-a}" \
          "${FAKE_SHARED_MANAGER:-memeloop-workspace-control}" "$workspace_label"
      else
        printf '{"metadata":{"labels":{"%s":"%s","app.kubernetes.io/managed-by":"%s"}}}' \
          "$owner_label" "${FAKE_SHARED_OWNER:-test-a}" \
          "${FAKE_SHARED_MANAGER:-memeloop-workspace-control}"
      fi
    else
      printf '{"metadata":{"labels":{"%s":"%s"}}}' \
        "$owner_label" "${FAKE_RELEASE_OWNER:-test-a}"
    fi
    return
  fi
  if [[ $1 == get && $2 == namespaces ]]; then
    if [[ ${FAKE_INCLUDE_SHARED:-false} == true ]]; then
      printf '{"items":[{"metadata":{"name":"mwc-test-a","labels":{"%s":"test-a"}}},{"metadata":{"name":"workspace-shared","labels":{"%s":"test-a","app.kubernetes.io/managed-by":"memeloop-workspace-control"}}},{"metadata":{"name":"ws-test-a-00000001","labels":{"%s":"test-a","%s":"canonical-workspace"}}}]}' \
        "$owner_label" "$owner_label" "$owner_label" "$workspace_label"
    elif [[ ${FAKE_INVALID_DEDICATED_NAMESPACE:-false} == true ]]; then
      printf '{"items":[{"metadata":{"name":"mwc-test-a","labels":{"%s":"test-a"}}},{"metadata":{"name":"foreign-name","labels":{"%s":"test-a","%s":"workspace"}}}]}' \
        "$owner_label" "$owner_label" "$workspace_label"
    else
      printf '{"items":[{"metadata":{"name":"mwc-test-a","labels":{"%s":"test-a"}}},{"metadata":{"name":"ws-test-a-00000001","labels":{"%s":"test-a","%s":"workspace"}}}]}' \
        "$owner_label" "$owner_label" "$workspace_label"
    fi
    return
  fi
  if [[ $1 == get && $2 == pvc ]]; then
    printf '{"items":[]}'
    return
  fi
  if [[ $1 == get && $2 == tcproute ]]; then
    return 1
  fi
  printf 'unexpected kubectl arguments: %q' "$@" >&2
  printf '\n' >&2
  return 2
}
export -f kubectl
export owner_label workspace_label

K3S_INSTALLATION_ID=test-a \
K3S_RELEASE_NAMESPACE=mwc-test-a \
K3S_MODE=sqlite \
K3S_WORKSPACE_NAMESPACE_SCOPE=dedicated \
bash "$verifier" >/dev/null

if FAKE_RELEASE_OWNER=other-installation \
  K3S_INSTALLATION_ID=test-a \
  K3S_RELEASE_NAMESPACE=mwc-test-a \
  K3S_MODE=sqlite \
  K3S_WORKSPACE_NAMESPACE_SCOPE=dedicated \
  bash "$verifier" >/dev/null 2>&1; then
  printf 'verification accepted a release namespace owned by another installation\n' >&2
  exit 1
fi

if FAKE_INVALID_DEDICATED_NAMESPACE=true \
  K3S_INSTALLATION_ID=test-a \
  K3S_RELEASE_NAMESPACE=mwc-test-a \
  K3S_MODE=sqlite \
  K3S_WORKSPACE_NAMESPACE_SCOPE=dedicated \
  bash "$verifier" >/dev/null 2>&1; then
  printf 'verification accepted a dedicated namespace without the installation prefix\n' >&2
  exit 1
fi

FAKE_INCLUDE_SHARED=true \
K3S_INSTALLATION_ID=test-a \
K3S_RELEASE_NAMESPACE=mwc-test-a \
K3S_MODE=sqlite \
K3S_WORKSPACE_NAMESPACE_SCOPE=shared \
K3S_WORKSPACE_SHARED_NAMESPACE=workspace-shared \
bash "$verifier" >/dev/null

pending_output=$(FAKE_SHARED_ABSENT=true \
  K3S_INSTALLATION_ID=test-a \
  K3S_RELEASE_NAMESPACE=mwc-test-a \
  K3S_MODE=sqlite \
  K3S_WORKSPACE_NAMESPACE_SCOPE=shared \
  K3S_WORKSPACE_SHARED_NAMESPACE=workspace-shared \
  bash "$verifier")
if [[ $pending_output != *"(shared:pending)"* ]]; then
  printf 'verification did not report an unmaterialized shared namespace as pending\n' >&2
  exit 1
fi

if FAKE_INCLUDE_SHARED=true \
  FAKE_SHARED_OWNER=other-installation \
  K3S_INSTALLATION_ID=test-a \
  K3S_RELEASE_NAMESPACE=mwc-test-a \
  K3S_MODE=sqlite \
  K3S_WORKSPACE_NAMESPACE_SCOPE=shared \
  K3S_WORKSPACE_SHARED_NAMESPACE=workspace-shared \
  bash "$verifier" >/dev/null 2>&1; then
  printf 'verification accepted a shared namespace owned by another installation\n' >&2
  exit 1
fi

if FAKE_INCLUDE_SHARED=true \
  FAKE_SHARED_WORKSPACE_LABEL=true \
  K3S_INSTALLATION_ID=test-a \
  K3S_RELEASE_NAMESPACE=mwc-test-a \
  K3S_MODE=sqlite \
  K3S_WORKSPACE_NAMESPACE_SCOPE=shared \
  K3S_WORKSPACE_SHARED_NAMESPACE=workspace-shared \
  bash "$verifier" >/dev/null 2>&1; then
  printf 'verification accepted a shared namespace carrying a workspace ID\n' >&2
  exit 1
fi

printf 'K3s installation verification tests passed\n'
