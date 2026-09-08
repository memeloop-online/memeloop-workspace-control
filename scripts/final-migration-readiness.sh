#!/usr/bin/env bash
# Read-only evidence collector for FINAL-MIGRATION-RUNBOOK.md.
# It intentionally contains no kubectl apply, delete, patch, scale, rollout, exec, or logs calls.
set -euo pipefail

readonly INSTALLATION_ID="${1:-k3si-7032544955}"
readonly CONTROL_NAMESPACE="mwc-${INSTALLATION_ID}"
readonly TARGET_NAMESPACE="memeloop-workspace-control"
readonly CODER_NAMESPACE="coder"
readonly CODER_PVC="coder-f5c0873c-8b1e-4099-b781-af5477688c39-rust-dev-home"
readonly WORKSPACE_NAMESPACES=(
  "ws-${INSTALLATION_ID}-bd2dc9ca6aa2b1b5"
  "ws-${INSTALLATION_ID}-b268ff46894a14b9"
  "ws-${INSTALLATION_ID}-97645a0fb4771b1d"
  "ws-${INSTALLATION_ID}-b405d2441a1ee149"
)

require_kubectl() { command -v kubectl >/dev/null || { echo "kubectl is required" >&2; exit 127; }; }
section() { printf '\n## %s\n' "$1"; }

require_kubectl
section "Context and namespace existence (read only)"
kubectl config current-context
for namespace in "$TARGET_NAMESPACE" "$CONTROL_NAMESPACE" "$CODER_NAMESPACE" "${WORKSPACE_NAMESPACES[@]}"; do
  kubectl get namespace "$namespace" \
    -o custom-columns='NAME:.metadata.name,PHASE:.status.phase,CREATED:.metadata.creationTimestamp' \
    2>/dev/null || printf 'MISSING  %s\n' "$namespace"
done

section "Control plane and MWC workspace PVC -> PV bindings"
kubectl get pvc -A -o custom-columns='NAMESPACE:.metadata.namespace,NAME:.metadata.name,PHASE:.status.phase,PV:.spec.volumeName,SC:.spec.storageClassName,CAPACITY:.status.capacity.storage,UID:.metadata.uid' \
  | { head -1; rg "^(${CONTROL_NAMESPACE}|${WORKSPACE_NAMESPACES[0]}|${WORKSPACE_NAMESPACES[1]}|${WORKSPACE_NAMESPACES[2]}|${WORKSPACE_NAMESPACES[3]})\\s"; }
kubectl -n "$CODER_NAMESPACE" get pvc "$CODER_PVC" \
  -o custom-columns='NAMESPACE:.metadata.namespace,NAME:.metadata.name,PHASE:.status.phase,PV:.spec.volumeName,SC:.spec.storageClassName,CAPACITY:.status.capacity.storage,UID:.metadata.uid'

section "PV claim references, reclaim policies, and CSI handles"
mapfile -t PVS < <(kubectl get pvc -A -o jsonpath='{range .items[?(@.spec.volumeName)]}{.metadata.namespace}{" "}{.metadata.name}{" "}{.spec.volumeName}{"\n"}{end}' \
  | awk -v c="$CONTROL_NAMESPACE" -v cn="$CODER_NAMESPACE" -v cp="$CODER_PVC" -v a="${WORKSPACE_NAMESPACES[0]}" -v b="${WORKSPACE_NAMESPACES[1]}" -v d="${WORKSPACE_NAMESPACES[2]}" -v e="${WORKSPACE_NAMESPACES[3]}" '$1==c || $1==a || $1==b || $1==d || $1==e || ($1==cn && $2==cp) {print $3}')
for pv in "${PVS[@]}"; do
  kubectl get pv "$pv" -o jsonpath='{.metadata.name}{"|reclaim="}{.spec.persistentVolumeReclaimPolicy}{"|claim="}{.spec.claimRef.namespace}{"/"}{.spec.claimRef.name}{"|claimUID="}{.spec.claimRef.uid}{"|driver="}{.spec.csi.driver}{"|handle="}{.spec.csi.volumeHandle}{"|phase="}{.status.phase}{"\n"}'
done

section "MWC workload ownership and readiness"
kubectl get pod -A -o custom-columns='NAMESPACE:.metadata.namespace,NAME:.metadata.name,READY:.status.containerStatuses[*].ready,PHASE:.status.phase,OWNER:.metadata.ownerReferences[0].kind' \
  | { head -1; rg "^(${CONTROL_NAMESPACE}|${WORKSPACE_NAMESPACES[0]}|${WORKSPACE_NAMESPACES[1]}|${WORKSPACE_NAMESPACES[2]}|${WORKSPACE_NAMESPACES[3]})\\s"; }
kubectl get statefulset -A -o custom-columns='NAMESPACE:.metadata.namespace,NAME:.metadata.name,READY:.status.readyReplicas,GENERATION:.metadata.generation' \
  | { head -1; rg "^(${CONTROL_NAMESPACE}|${WORKSPACE_NAMESPACES[0]}|${WORKSPACE_NAMESPACES[1]}|${WORKSPACE_NAMESPACES[2]}|${WORKSPACE_NAMESPACES[3]})\\s"; }

section "Independent Coder Pod -> PVC chain"
kubectl -n "$CODER_NAMESPACE" get pod -o jsonpath='{range .items[*]}{.metadata.name}{"\t"}{.status.phase}{"\t"}{range .spec.volumes[?(@.persistentVolumeClaim)]}{.persistentVolumeClaim.claimName}{","}{end}{"\n"}{end}' \
  | awk -v claim="$CODER_PVC" '$3 ~ claim {print}'

section "Longhorn attachment and health (no volume data is read)"
printf '%-40s %-10s %-12s %-14s %-40s %s\n' NAME STATE ROBUSTNESS NODE PV WORKLOAD
for pv in "${PVS[@]}"; do
  driver=$(kubectl get pv "$pv" -o jsonpath='{.spec.csi.driver}')
  handle=$(kubectl get pv "$pv" -o jsonpath='{.spec.csi.volumeHandle}')
  [[ "$driver" == 'driver.longhorn.io' ]] || continue
  kubectl -n longhorn-system get volumes.longhorn.io "$handle" \
    -o custom-columns='NAME:.metadata.name,STATE:.status.state,ROBUSTNESS:.status.robustness,NODE:.status.currentNodeID,PV:.status.kubernetesStatus.pvName,WORKLOAD:.status.kubernetesStatus.workloadsStatus[0].workloadName' \
    --no-headers || printf 'MISSING LONGHORN VOLUME %s\n' "$handle"
done

echo 'Read-only report complete. Target namespace MISSING is expected before cutover; treat missing source objects, degraded/faulted volumes, unexpected attachments, or same-chain claim-UID mismatch as stop gates.'
