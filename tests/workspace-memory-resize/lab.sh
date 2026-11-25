#!/usr/bin/env bash
# Use only the disposable GitHub-hosted runner; this starts its own K3s.
set -Eeuo pipefail
test "${GITHUB_ACTIONS:-}" = true
test "${RUNNER_ENVIRONMENT:-}" = github-hosted
dir=$1
out=$dir/evidence
k3s=$dir/k3s
image=docker.io/library/mwc-memory-ci:fixture
mkdir -p "$out" "$dir/build"
k() { sudo "$k3s" kubectl --kubeconfig "$dir/kubeconfig" --request-timeout=8s "$@"; }
cri() { sudo env K3S_DATA_DIR="$dir/data" "$k3s" crictl "$@"; }
cleanup() {
  local code=$?
  if [[ -n ${heartbeat:-} ]]; then kill "$heartbeat" 2>/dev/null || true; fi
  if [[ -n ${load:-} ]]; then kill "$load" 2>/dev/null || true; fi
  if ((code != 0)); then
    sudo journalctl -u mwc-memory-ci -n 70 --no-pager > "$out/k3s.log" || true
    k get events -A > "$out/events.txt" || true
    k describe pod workspace-0 > "$out/pod-diagnostics.txt" || true
  fi
  sudo systemctl stop mwc-memory-ci || true
}
trap cleanup EXIT
trap 'printf "memory resize lab failed at line %s\n" "$LINENO" >&2' ERR

curl -fsSL --retry 2 -o "$k3s" https://github.com/k3s-io/k3s/releases/download/v1.36.2%2Bk3s1/k3s
printf '%s  %s\n' 65a55ec56c24eab44383086166ec620a491952b7e23941a49ddca6e8a4c4b4de "$k3s" | sha256sum -c
chmod +x "$k3s"
gcc -O2 -static -Werror -Wall -Wextra "$(dirname "$0")/pressure.c" -o "$dir/build/pressure"
cat > "$dir/build/Dockerfile" <<'DOCKERFILE'
FROM alpine:3.22.1@sha256:4bcff63911fcb4448bd4fdacec207030997caf25e9bea4045fa6c8c44de311d1
RUN apk add --no-cache openssh-server && adduser -D mwcci && passwd -d mwcci
COPY pressure /usr/local/bin/pressure
CMD ["sh","-ec","install -d -o mwcci -g mwcci -m 0700 /home/mwcci/.ssh; install -o mwcci -g mwcci -m 0600 /ssh/authorized_keys /home/mwcci/.ssh/authorized_keys; exec /usr/sbin/sshd -De -h /ssh/host -o PasswordAuthentication=no -o KbdInteractiveAuthentication=no"]
DOCKERFILE
docker build --tag "$image" "$dir/build"
docker save -o "$dir/image.tar" "$image"
ssh-keygen -q -t ed25519 -N '' -f "$dir/client"
ssh-keygen -q -t ed25519 -N '' -f "$dir/host"
sudo systemd-run --unit=mwc-memory-ci --property=Delegate=yes "$k3s" server \
  --data-dir "$dir/data" --write-kubeconfig "$dir/kubeconfig" --node-name=mwc-memory-ci \
  --prefer-bundled-bin --flannel-backend=host-gw --disable-network-policy \
  --disable=coredns,local-storage,metrics-server,servicelb,traefik
for _ in {1..60}; do
  if k get node mwc-memory-ci >/dev/null 2>&1; then break; fi
  sleep 1
done
k wait --for=condition=Ready node/mwc-memory-ci --timeout=120s
cri version > "$out/cri-version.txt"
grep -Eq '^RuntimeVersion:[[:space:]]+v?2\.3\.2-k3s2$' "$out/cri-version.txt"
sudo env K3S_DATA_DIR="$dir/data" "$k3s" ctr --address /run/k3s/containerd/containerd.sock \
  --namespace k8s.io images import "$dir/image.tar"
k create secret generic fixture-ssh --from-file="host=$dir/host" --from-file="authorized_keys=$dir/client.pub"
k create -f - <<EOF
apiVersion: v1
kind: Service
metadata:
  name: workspace
spec:
  clusterIP: None
  selector: {app: workspace}
  ports: [{port: 22}]
---
apiVersion: apps/v1
kind: StatefulSet
metadata:
  name: workspace
spec:
  serviceName: workspace
  selector:
    matchLabels: {app: workspace}
  template:
    metadata:
      labels: {app: workspace}
    spec:
      automountServiceAccountToken: false
      containers:
        - name: workspace
          image: $image
          imagePullPolicy: Never
          ports: [{containerPort: 22}]
          readinessProbe:
            tcpSocket: {port: 22}
          resources:
            requests: {cpu: 100m, memory: 64Mi}
            limits: {cpu: "2", memory: 256Mi}
          volumeMounts: [{name: ssh, mountPath: /ssh, readOnly: true}]
      volumes:
        - name: ssh
          secret: {secretName: fixture-ssh, defaultMode: 256}
EOF

snapshot() {
  k get pod workspace-0 -o json | jq '{
    uid:.metadata.uid,ip:.status.podIP,qos:.status.qosClass,
    containerID:.status.containerStatuses[0].containerID,
    restarts:.status.containerStatuses[0].restartCount,
    wanted:.spec.containers[0].resources.limits.memory,
    applied:.status.containerStatuses[0].resources.limits.memory,
    revision:.metadata.labels["controller-revision-hash"]
  }'
}
set_host() {
  address=$(jq -er .ip "$1")
  printf '%s %s\n' "$address" "$(cut -d' ' -f1,2 "$dir/host.pub")" > "$dir/known"
}
ssh_fixture() {
  ssh -F /dev/null -o BatchMode=yes -o StrictHostKeyChecking=yes \
    -o "UserKnownHostsFile=$dir/known" -o ConnectTimeout=3 -i "$dir/client" "mwcci@$address" "$@"
}
k rollout status statefulset/workspace --timeout=90s
snapshot > "$out/before.json"
jq -e '.qos=="Burstable" and .applied=="256Mi" and .restarts==0' "$out/before.json"
set_host "$out/before.json"
test "$(ssh_fixture 'printf baseline')" = baseline
timeout 180 ssh -F /dev/null -o BatchMode=yes -o StrictHostKeyChecking=yes \
  -o "UserKnownHostsFile=$dir/known" -o ConnectTimeout=3 -i "$dir/client" "mwcci@$address" \
  'while :; do date +%s; sleep 0.2; done' > "$out/heartbeat.txt" &
heartbeat=$!

container=$(jq -er '.containerID|sub("^containerd://";"")' "$out/before.json")
pid=$(cri inspect "$container" | jq -er '.info.pid')
cgroup=$(sudo awk -F: '$1=="0"{print $3}' "/proc/$pid/cgroup")
test "$cgroup" != /
test -n "$cgroup"
cgroup=/sys/fs/cgroup$cgroup
event_max() { sudo awk '$1=="max"{print $2}' "$cgroup/memory.events"; }
max_before=$(event_max)
k --request-timeout=0 exec workspace-0 -- /usr/local/bin/pressure > "$out/pressure.log" 2>&1 &
load=$!
# Exec uses the workspace's cgroup. Wait for observed max events, not a sleep-based assumption.
for _ in {1..60}; do
  kill -0 "$load"
  if (( $(event_max) > max_before )); then break; fi
  sleep 1
done
max_under_pressure=$(event_max)
test "$max_under_pressure" -gt "$max_before"
sudo cat "$cgroup/memory.pressure" > "$out/memory-pressure-before.txt"
sudo cat "$cgroup/memory.events" > "$out/memory-events-before.txt"

# Match the product's guarded patches. Dry-run the actual subresource before
# persisting the replacement template; a stale request cannot touch this Pod.
uid=$(jq -er .uid "$out/before.json")
revision=$(jq -er .revision "$out/before.json")
memory='{"name":"workspace","resources":{"limits":{"memory":"512Mi"}}}'
pod_rv=$(k get pod workspace-0 -o jsonpath='{.metadata.resourceVersion}')
pod_patch=$(jq -cn --arg uid "$uid" --arg rv "$pod_rv" --argjson memory "$memory"   '{metadata:{uid:$uid,resourceVersion:$rv},spec:{containers:[$memory]}}')
k patch pod workspace-0 --subresource=resize --type=strategic --field-manager=mwc-control-plane --dry-run=server -p "$pod_patch"
bad_uid_patch=$(jq '.metadata.uid="970ab56c-8884-4a49-93a7-40926243dc89"' <<< "$pod_patch")
if k patch pod workspace-0 --subresource=resize --type=strategic --field-manager=mwc-control-plane --dry-run=server -p "$bad_uid_patch" > "$out/stale-uid.txt" 2>&1; then
  echo "resize dry-run accepted the wrong Pod UID" >&2
  exit 1
fi
set_identity=$(k get statefulset workspace -o json | jq '.metadata | {uid,resourceVersion}')
set_patch=$(jq -cn --argjson identity "$set_identity" --arg revision "$revision" --argjson memory "$memory" '
  {metadata:($identity + {annotations:{"workspace.memeloop.dev/memory-resize-retained-revision":$revision}}),
    spec:{updateStrategy:{type:"OnDelete",rollingUpdate:null},template:{spec:{containers:[$memory]}}}}')
k patch statefulset workspace --type=strategic --field-manager=mwc-control-plane -p "$set_patch"
test "$(k get pod workspace-0 -o jsonpath='{.metadata.uid}')" = "$uid"
# Status-only updates can advance resourceVersion. The worker retries from a
# fresh read: never drop the UID/resourceVersion precondition.
resize_started=$(date +%s%3N)
for _ in {1..12}; do
  pod_rv=$(k get pod workspace-0 -o jsonpath='{.metadata.resourceVersion}')
  pod_patch=$(jq --arg rv "$pod_rv" '.metadata.resourceVersion=$rv' <<< "$pod_patch")
  if k patch pod workspace-0 --subresource=resize --type=strategic --field-manager=mwc-control-plane -p "$pod_patch"; then
    break
  fi
  sleep 0.25
done
for _ in {1..40}; do
  snapshot > "$out/after.json"
  if jq -e '.applied=="512Mi"' "$out/after.json" >/dev/null; then break; fi
  sleep 1
done
jq -e --slurpfile before "$out/before.json" '
  .applied=="512Mi" and .wanted=="512Mi" and
  ([.uid,.containerID,.restarts,.revision]==($before[0]|[.uid,.containerID,.restarts,.revision]))
' "$out/after.json"
test "$(sudo cat "$cgroup/memory.max")" = 536870912
resize_applied_ms=$(($(date +%s%3N)-resize_started))
kill -0 "$load"
sudo cat "$cgroup/memory.pressure" > "$out/memory-pressure-after.txt"
sudo cat "$cgroup/memory.events" > "$out/memory-events-after.txt"
heartbeats=$(wc -l < "$out/heartbeat.txt")
for _ in {1..20}; do
  kill -0 "$heartbeat"
  if (( $(wc -l < "$out/heartbeat.txt") > heartbeats )); then break; fi
  sleep 0.5
done
test "$(wc -l < "$out/heartbeat.txt")" -gt "$heartbeats"
test "$(ssh_fixture 'printf new-auth')" = new-auth
k exec workspace-0 -- touch /tmp/pressure-stop
wait "$load"
load=
kill "$heartbeat"
heartbeat=

k delete pod workspace-0 --wait=true --timeout=45s
for _ in {1..30}; do
  if k get pod workspace-0 >/dev/null 2>&1; then break; fi
  sleep 1
done
k wait --for=condition=Ready pod/workspace-0 --timeout=90s
snapshot > "$out/replacement.json"
jq -e --arg uid "$uid" --slurpfile before "$out/before.json" '
  .uid!=$uid and .applied=="512Mi" and .wanted=="512Mi" and .revision!=$before[0].revision
' "$out/replacement.json"
set_host "$out/replacement.json"
test "$(ssh_fixture 'printf replacement')" = replacement
jq -n --argjson under "$max_under_pressure" --argjson before "$max_before" --argjson elapsed "$resize_applied_ms" \
  '{accepted:true,memory_limit_events_before_resize:($under-$before),resize_applied_ms:$elapsed,same_pod_and_container:true,same_ssh_session_progressed:true,new_strict_ssh:true,replacement_uses_desired_limit:true,production_changes:false}' > "$out/acceptance.json"
