#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
preflight="$repo_root/scripts/k3s/preflight.sh"

kubectl() {
  if [[ $1 == config && $2 == current-context ]]; then
    printf 'k3s-test\n'
    return
  fi
  if [[ $1 == config && $2 == view ]]; then
    printf 'https://kubernetes.test\n'
    return
  fi
  if [[ $1 == version ]]; then
    return
  fi
  if [[ $1 == auth && $2 == can-i ]]; then
    printf 'yes\n'
    return
  fi
  if [[ $1 == get && $2 == storageclass ]]; then
    printf 'Delete\n'
    return
  fi
  if [[ $1 == get && $2 == namespace ]]; then
    [[ ${FAKE_NAMESPACE_EXISTS:-false} == true ]]
    return
  fi
  if [[ $1 == get && $2 == deployment,statefulset,service,serviceaccount,configmap,secret,pvc,networkpolicy,ingress ]]; then
    printf '%s' "${FAKE_OWNER_OUTPUT:-}"
    return
  fi
  if [[ $1 == get && $2 == httproute ]]; then
    printf '%s' "${FAKE_ROUTE_OWNER_OUTPUT:-}"
    return
  fi
  if [[ $1 == get && $2 == gateway ]]; then
    printf '%s' "${FAKE_LISTENER_PORTS:-}"
    return
  fi
  if [[ $1 == get && $2 == pods ]]; then
    printf '%s' "${FAKE_GATEWAY_PODS:-}"
    return
  fi
  if [[ $1 == get && $2 == crd ]]; then
    if [[ $3 == httproutes.gateway.networking.k8s.io && ${FAKE_HTTPROUTE_CRD:-true} != true ]]; then
      return 1
    fi
    return
  fi
  printf 'unexpected kubectl arguments: %q' "$@" >&2
  printf '\n' >&2
  return 2
}
export -f kubectl

run_preflight() {
  K3S_INSTALLATION_ID=test-a \
  K3S_RELEASE_NAMESPACE=mwc-test-a \
  K3S_STORAGE_CLASS=managed-delete \
  K3S_MODE=${K3S_MODE:-sqlite} \
  K3S_PUBLIC_SSH=${K3S_PUBLIC_SSH:-false} \
  K3S_PUBLIC_WEB_SHELL=${K3S_PUBLIC_WEB_SHELL:-false} \
  K3S_PUBLIC_API=${K3S_PUBLIC_API:-false} \
  K3S_HIGRESS_NAMESPACE=${K3S_HIGRESS_NAMESPACE:-higress-system} \
  K3S_HIGRESS_GATEWAY=${K3S_HIGRESS_GATEWAY:-higress-gateway} \
  bash "$preflight"
}

FAKE_NAMESPACE_EXISTS=false FAKE_HTTPROUTE_CRD=false run_preflight >/dev/null

FAKE_NAMESPACE_EXISTS=false \
FAKE_HTTPROUTE_CRD=false \
FAKE_GATEWAY_PODS='gateway-a' \
K3S_PUBLIC_WEB_SHELL=true \
run_preflight >/dev/null

if FAKE_NAMESPACE_EXISTS=false \
  FAKE_HTTPROUTE_CRD=false \
  K3S_PUBLIC_API=true \
  run_preflight >/dev/null 2>&1; then
  printf 'preflight accepted a public API without HTTPRoute CRDs\n' >&2
  exit 1
fi

FAKE_NAMESPACE_EXISTS=true \
FAKE_OWNER_OUTPUT=$'test-a\ntest-a\n' \
run_preflight >/dev/null

if FAKE_NAMESPACE_EXISTS=true \
  FAKE_OWNER_OUTPUT=$'test-a\nother-installation\n' \
  run_preflight >/dev/null 2>&1; then
  printf 'preflight accepted a namespace owned by another installation\n' >&2
  exit 1
fi

FAKE_NAMESPACE_EXISTS=false \
FAKE_LISTENER_PORTS=$'80\n22\n443\n' \
FAKE_GATEWAY_PODS=$'gateway-a\ngateway-b\n' \
K3S_PUBLIC_SSH=true \
K3S_PUBLIC_WEB_SHELL=true \
run_preflight >/dev/null

if FAKE_NAMESPACE_EXISTS=false \
  FAKE_LISTENER_PORTS=$'22\n22\n' \
  K3S_PUBLIC_SSH=true \
  run_preflight >/dev/null 2>&1; then
  printf 'preflight accepted multiple TCP 22 listeners\n' >&2
  exit 1
fi

if FAKE_NAMESPACE_EXISTS=false \
  FAKE_GATEWAY_PODS='' \
  K3S_PUBLIC_WEB_SHELL=true \
  run_preflight >/dev/null 2>&1; then
  printf 'preflight accepted an empty Higress gateway selector\n' >&2
  exit 1
fi

printf 'K3S preflight tests passed\n'
