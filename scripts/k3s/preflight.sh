#!/usr/bin/env bash
set -euo pipefail

required() {
  local name=$1
  if [[ -z ${!name:-} ]]; then
    printf 'required environment variable is empty: %s\n' "$name" >&2
    exit 64
  fi
}

required K3S_INSTALLATION_ID
required K3S_RELEASE_NAMESPACE
required K3S_STORAGE_CLASS
required K3S_MODE

if [[ ! $K3S_INSTALLATION_ID =~ ^[a-z0-9]([-a-z0-9]{0,18}[a-z0-9])?$ ]]; then
  printf 'K3S_INSTALLATION_ID must be a lower-case DNS label of at most 20 characters\n' >&2
  exit 64
fi
if [[ $K3S_MODE != sqlite && $K3S_MODE != postgresql ]]; then
  printf 'K3S_MODE must be sqlite or postgresql\n' >&2
  exit 64
fi

command -v kubectl >/dev/null

context=$(kubectl config current-context)
server=$(kubectl config view --minify -o jsonpath='{.clusters[0].cluster.server}')
printf 'Kubernetes context: %s\nAPI server: %s\n' "$context" "$server"
kubectl version --request-timeout=10s >/dev/null

reclaim_policy=$(kubectl get storageclass "$K3S_STORAGE_CLASS" -o jsonpath='{.reclaimPolicy}')
if [[ $reclaim_policy != Delete ]]; then
  printf 'StorageClass %s has reclaimPolicy=%s; managed workspace storage requires Delete\n' \
    "$K3S_STORAGE_CLASS" "$reclaim_policy" >&2
  exit 1
fi

for resource in namespaces clusterroles.rbac.authorization.k8s.io clusterrolebindings.rbac.authorization.k8s.io; do
  if [[ $(kubectl auth can-i create "$resource") != yes ]]; then
    printf 'current identity cannot create %s\n' "$resource" >&2
    exit 1
  fi
done
for resource in serviceaccounts deployments.apps statefulsets.apps services secrets configmaps persistentvolumeclaims networkpolicies.networking.k8s.io; do
  if [[ $(kubectl auth can-i create "$resource" --namespace "$K3S_RELEASE_NAMESPACE") != yes ]]; then
    printf 'current identity cannot create %s in %s\n' "$resource" "$K3S_RELEASE_NAMESPACE" >&2
    exit 1
  fi
done
if [[ $K3S_MODE == postgresql ]] && [[ $(kubectl auth can-i create horizontalpodautoscalers.autoscaling --namespace "$K3S_RELEASE_NAMESPACE") != yes ]]; then
  printf 'current identity cannot create horizontalpodautoscalers.autoscaling in %s\n' "$K3S_RELEASE_NAMESPACE" >&2
  exit 1
fi

if kubectl get namespace "$K3S_RELEASE_NAMESPACE" >/dev/null 2>&1; then
  owners=$(kubectl get deployment,statefulset,service,serviceaccount,configmap,secret,pvc,networkpolicy \
    -n "$K3S_RELEASE_NAMESPACE" \
    -o go-template='{{range .items}}{{if .metadata.labels}}{{with index .metadata.labels "workspace.memeloop.dev/owner-installation"}}{{printf "%s\n" .}}{{end}}{{end}}{{end}}' \
    2>/dev/null)
  if kubectl get crd httproutes.gateway.networking.k8s.io >/dev/null 2>&1; then
    route_owners=$(kubectl get httproute -n "$K3S_RELEASE_NAMESPACE" \
      -o go-template='{{range .items}}{{if .metadata.labels}}{{with index .metadata.labels "workspace.memeloop.dev/owner-installation"}}{{printf "%s\n" .}}{{end}}{{end}}{{end}}' \
      2>/dev/null)
    owners+=$'\n'"$route_owners"
  fi
  while IFS= read -r owner; do
    if [[ -n $owner && $owner != "$K3S_INSTALLATION_ID" ]]; then
      printf 'release namespace contains resources owned by another installation\n' >&2
      exit 1
    fi
  done <<< "$owners"
fi

public_ssh=${K3S_PUBLIC_SSH:-false}
public_web_shell=${K3S_PUBLIC_WEB_SHELL:-false}
public_api=${K3S_PUBLIC_API:-false}
if [[ $public_ssh == true || $public_web_shell == true || $public_api == true ]]; then
  required K3S_HIGRESS_NAMESPACE
  required K3S_HIGRESS_GATEWAY
  kubectl get crd gateways.gateway.networking.k8s.io >/dev/null
  kubectl get crd httproutes.gateway.networking.k8s.io >/dev/null
  kubectl get gateway -n "$K3S_HIGRESS_NAMESPACE" "$K3S_HIGRESS_GATEWAY" >/dev/null
fi
if [[ $public_web_shell == true || $public_api == true ]]; then
  if [[ $(kubectl auth can-i create httproutes.gateway.networking.k8s.io --namespace "$K3S_RELEASE_NAMESPACE") != yes ]]; then
    printf 'current identity cannot create HTTPRoute in %s\n' "$K3S_RELEASE_NAMESPACE" >&2
    exit 1
  fi
fi
if [[ $public_ssh == true ]]; then
  kubectl get crd tcproutes.gateway.networking.k8s.io >/dev/null
  listener_ports=$(kubectl get gateway -n "$K3S_HIGRESS_NAMESPACE" "$K3S_HIGRESS_GATEWAY" \
    -o go-template='{{range .spec.listeners}}{{printf "%v\n" .port}}{{end}}')
  listener_count=0
  while IFS= read -r port; do
    if [[ $port == 22 ]]; then
      ((listener_count += 1))
    fi
  done <<< "$listener_ports"
  if [[ $listener_count != 1 ]]; then
    printf 'Higress Gateway must expose exactly one listener on port 22; found %s\n' "$listener_count" >&2
    exit 1
  fi
  if [[ $(kubectl auth can-i create tcproutes.gateway.networking.k8s.io --namespace "$K3S_RELEASE_NAMESPACE") != yes ]]; then
    printf 'current identity cannot create TCPRoute in %s\n' "$K3S_RELEASE_NAMESPACE" >&2
    exit 1
  fi
fi
if [[ $public_web_shell == true ]]; then
  kubectl get crd wasmplugins.extensions.higress.io >/dev/null
  selector=${K3S_HIGRESS_POD_SELECTOR:-app.kubernetes.io/name=higress-gateway}
  gateway_pod_names=$(kubectl get pods -n "$K3S_HIGRESS_NAMESPACE" -l "$selector" \
    -o go-template='{{range .items}}{{printf "%s\n" .metadata.name}}{{end}}')
  gateway_pods=0
  while IFS= read -r pod_name; do
    if [[ -n $pod_name ]]; then
      ((gateway_pods += 1))
    fi
  done <<< "$gateway_pod_names"
  if [[ $gateway_pods == 0 ]]; then
    printf 'no Higress gateway Pods match selector %s in %s\n' "$selector" "$K3S_HIGRESS_NAMESPACE" >&2
    exit 1
  fi
  if [[ $(kubectl auth can-i create wasmplugins.extensions.higress.io --namespace "$K3S_HIGRESS_NAMESPACE") != yes ]]; then
    printf 'current identity cannot create WasmPlugin in %s\n' "$K3S_HIGRESS_NAMESPACE" >&2
    exit 1
  fi
fi

printf 'K3S preflight passed for installation %s (%s) in namespace %s\n' \
  "$K3S_INSTALLATION_ID" "$K3S_MODE" "$K3S_RELEASE_NAMESPACE"
