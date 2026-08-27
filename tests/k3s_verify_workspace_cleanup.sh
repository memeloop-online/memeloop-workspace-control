#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
verifier="$repo_root/scripts/k3s/verify-workspace-cleanup.sh"

kubectl() {
  if [[ $1 == get && $2 == namespace ]]; then
    [[ ${FAKE_NAMESPACE_EXISTS:-false} == true ]]
    return
  fi
  if [[ $1 == get && $2 == crd ]]; then
    return 1
  fi
  if [[ $1 == get && $2 == statefulset,service,secret,configmap,pvc,networkpolicy,ingress ]]; then
    printf '{"items":[]}'
    return
  fi
  printf 'unexpected kubectl arguments: %q' "$@" >&2
  printf '\n' >&2
  return 2
}
export -f kubectl

K3S_INSTALLATION_ID=test-a \
K3S_WORKSPACE_ID=00000000-0000-0000-0000-000000000001 \
K3S_WORKSPACE_NAMESPACE=ws-test-a-00000001 \
bash "$verifier" >/dev/null

if FAKE_NAMESPACE_EXISTS=true \
  K3S_INSTALLATION_ID=test-a \
  K3S_WORKSPACE_ID=00000000-0000-0000-0000-000000000001 \
  K3S_WORKSPACE_NAMESPACE=ws-test-a-00000001 \
  bash "$verifier" >/dev/null 2>&1; then
  printf 'cleanup verification accepted an existing workspace namespace\n' >&2
  exit 1
fi

printf 'K3s workspace cleanup verification tests passed\n'
