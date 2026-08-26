#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
preflight="$repo_root/scripts/kcs/preflight.sh"

kubectl() {
  if [[ $1 == config && $2 == current-context ]]; then
    printf 'kcs-test\n'
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
  if [[ $1 == get && $2 == all,configmap,secret,pvc,networkpolicy,httproute ]]; then
    printf '%s' "${FAKE_OWNER_OUTPUT:-}"
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
    return
  fi
  printf 'unexpected kubectl arguments: %q' "$@" >&2
  printf '\n' >&2
  return 2
}
export -f kubectl

run_preflight() {
  KCS_INSTALLATION_ID=test-a \
  KCS_RELEASE_NAMESPACE=mwc-test-a \
  KCS_STORAGE_CLASS=managed-delete \
  KCS_MODE=${KCS_MODE:-sqlite} \
  KCS_PUBLIC_SSH=${KCS_PUBLIC_SSH:-false} \
  KCS_PUBLIC_WEB_SHELL=${KCS_PUBLIC_WEB_SHELL:-false} \
  KCS_HIGRESS_NAMESPACE=${KCS_HIGRESS_NAMESPACE:-higress-system} \
  KCS_HIGRESS_GATEWAY=${KCS_HIGRESS_GATEWAY:-higress-gateway} \
  bash "$preflight"
}

FAKE_NAMESPACE_EXISTS=false run_preflight >/dev/null

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
KCS_PUBLIC_SSH=true \
KCS_PUBLIC_WEB_SHELL=true \
run_preflight >/dev/null

if FAKE_NAMESPACE_EXISTS=false \
  FAKE_LISTENER_PORTS=$'22\n22\n' \
  KCS_PUBLIC_SSH=true \
  run_preflight >/dev/null 2>&1; then
  printf 'preflight accepted multiple TCP 22 listeners\n' >&2
  exit 1
fi

if FAKE_NAMESPACE_EXISTS=false \
  FAKE_GATEWAY_PODS='' \
  KCS_PUBLIC_WEB_SHELL=true \
  run_preflight >/dev/null 2>&1; then
  printf 'preflight accepted an empty Higress gateway selector\n' >&2
  exit 1
fi

printf 'KCS preflight tests passed\n'
