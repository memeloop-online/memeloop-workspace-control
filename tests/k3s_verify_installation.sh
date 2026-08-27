#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
verifier="$repo_root/scripts/k3s/verify-installation.sh"
owner_label='workspace.memeloop.dev/owner-installation'

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
  if [[ $1 == get && $2 == endpoints ]]; then
    printf '{"subsets":[{"addresses":[{}]}]}'
    return
  fi
  if [[ $1 == get && $2 == namespace ]]; then
    printf '{"metadata":{"labels":{"%s":"%s"}}}' "$owner_label" "${FAKE_RELEASE_OWNER:-test-a}"
    return
  fi
  if [[ $1 == get && $2 == namespaces ]]; then
    printf '{"items":[{"metadata":{"name":"mwc-test-a"}},{"metadata":{"name":"ws-test-a-00000001"}}]}'
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

K3S_INSTALLATION_ID=test-a \
K3S_RELEASE_NAMESPACE=mwc-test-a \
K3S_MODE=sqlite \
bash "$verifier" >/dev/null

if FAKE_RELEASE_OWNER=other-installation \
  K3S_INSTALLATION_ID=test-a \
  K3S_RELEASE_NAMESPACE=mwc-test-a \
  K3S_MODE=sqlite \
  bash "$verifier" >/dev/null 2>&1; then
  printf 'verification accepted a release namespace owned by another installation\n' >&2
  exit 1
fi

printf 'K3s installation verification tests passed\n'
