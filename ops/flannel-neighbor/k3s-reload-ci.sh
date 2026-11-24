#!/usr/bin/env bash
# Run only on the disposable GitHub-hosted runner. This owns its K3s, CNI and IPAM state.
set -euo pipefail
: "${MWC_K3S_CI_DIR:?A dedicated runner directory is required}"
dir=$MWC_K3S_CI_DIR
plugin=$1
unit=mwc-k3s-cni-ci
node=mwc-cni-ci
version=v1.36.2%2Bk3s1
k3s=$dir/k3s
kubeconfig=$dir/kubeconfig.yaml
runtime_root=$dir/data
mkdir -p "$dir"
test "${GITHUB_ACTIONS:-}" = true
test "${RUNNER_ENVIRONMENT:-}" = github-hosted

curl -fsSL --retry 2 -o "$k3s" "https://github.com/k3s-io/k3s/releases/download/$version/k3s"
printf '%s  %s\n' 65a55ec56c24eab44383086166ec620a491952b7e23941a49ddca6e8a4c4b4de "$k3s" | sha256sum -c
chmod +x "$k3s"

image=docker.io/library/mwc-k3s-ci-sshd:fixture
docker build --tag "$image" - <<'DOCKERFILE'
FROM alpine:3.22.1@sha256:4bcff63911fcb4448bd4fdacec207030997caf25e9bea4045fa6c8c44de311d1
RUN apk add --no-cache openssh-server && adduser -D mwcci && passwd -d mwcci
CMD ["sh", "-ec", "ssh-keygen -A; exec /usr/sbin/sshd -De -o PasswordAuthentication=no -o KbdInteractiveAuthentication=no -o AuthorizedKeysFile=/ssh/authorized_keys"]
DOCKERFILE
docker save -o "$dir/ssh.tar" "$image"
ssh-keygen -q -t ed25519 -N '' -f "$dir/client"
cat > "$dir/k3s.yaml" <<EOF
data-dir: $runtime_root
write-kubeconfig: $kubeconfig
node-name: $node
prefer-bundled-bin: true
flannel-backend: host-gw
disable: [coredns, local-storage, metrics-server, servicelb, traefik]
disable-network-policy: true
EOF

heartbeat_pid=
k() { sudo "$k3s" kubectl --kubeconfig "$kubeconfig" "$@"; }
cri() { sudo env K3S_DATA_DIR="$runtime_root" "$k3s" crictl "$@"; }
cleanup() {
  local code=$?
  if [[ -n $heartbeat_pid ]]; then kill "$heartbeat_pid" 2>/dev/null || true; fi
  if ((code != 0)); then
    sudo journalctl -q -u "$unit" -n 60 --no-pager || true
    timeout 10 sudo "$k3s" kubectl --kubeconfig "$kubeconfig" get events -A || true
  fi
  sudo systemctl stop "$unit" || true
}
trap cleanup EXIT
sudo systemd-run --unit "$unit" --property=Delegate=yes "$k3s" server --config "$dir/k3s.yaml"
for _ in {1..60}; do
  if k get node "$node" --request-timeout=2s >/dev/null 2>&1; then break; fi
  sleep 1
done
k wait --for=condition=Ready "node/$node" --timeout=120s
cri version | tee "$dir/cri-version.txt"
grep -Eq '^RuntimeVersion:[[:space:]]+v?2\.3\.2-k3s2$' "$dir/cri-version.txt"
sudo env K3S_DATA_DIR="$runtime_root" "$k3s" ctr --address /run/k3s/containerd/containerd.sock \
  --namespace k8s.io images import "$dir/ssh.tar"
k create configmap ssh --from-file="authorized_keys=$dir/client.pub"
node_ip=$(k get node "$node" -o json | jq -er '.status.addresses[] | select(.type == "InternalIP" and (.address | contains(":") | not)).address')

fixture() {
  local name=$1 port=${2:-}
  jq -n --arg name "$name" --arg image "$image" '
    {apiVersion:"v1", kind:"Pod", metadata:{name:$name,labels:{fixture:$name}},
     spec:{containers:[{name:"ssh",image:$image,imagePullPolicy:"Never",
            ports:[{containerPort:22}],volumeMounts:[{name:"ssh",mountPath:"/ssh",readOnly:true}]}],
           volumes:[{name:"ssh",configMap:{name:"ssh"}}]}}' | k create -f -
  if [[ -n $port ]]; then
    jq -n --arg name "$name" --argjson port "$port" '
      {apiVersion:"v1",kind:"Service",metadata:{name:$name},spec:{type:"NodePort",
       selector:{fixture:$name},ports:[{port:22,targetPort:22,nodePort:$port}]}}' | k create -f -
  fi
  k wait --for=condition=Ready "pod/$name" --timeout=90s
}
fixture_key() {
  local name=$1 key
  for _ in {1..20}; do
    if key=$(k exec "$name" -- cat /etc/ssh/ssh_host_ed25519_key.pub 2>/dev/null); then
      printf '%s %s\n' "$name" "$key" >> "$dir/known_hosts"
      return
    fi
    sleep 0.25
  done
  return 1
}
strict_ssh() {
  local name=$1 port=$2
  shift 2
  ssh -o BatchMode=yes -o ConnectTimeout=2 -o IdentitiesOnly=yes -i "$dir/client" -o ServerAliveInterval=1 \
    -o StrictHostKeyChecking=yes -o "UserKnownHostsFile=$dir/known_hosts" -o GlobalKnownHostsFile=/dev/null \
    -o "HostKeyAlias=$name" -p "$port" "mwcci@$node_ip" "$@"
}
await_ssh() {
  for _ in {1..25}; do
    if [[ $(strict_ssh "$1" "$2" hostname) == "$1" ]]; then return; fi
    sleep 0.25
  done
  return 1
}
pod_identity() {
  k get pod "$1" -o json | jq -cer '[.metadata.uid,.status.podIP,.status.containerStatuses[].containerID]'
}
containerd_pid() {
  local pid exe
  for pid in $(sudo pgrep -x containerd); do
    exe=$(sudo readlink "/proc/$pid/exe") || continue
    if [[ $exe == "$runtime_root"/data/*/bin/containerd ]]; then printf '%s\n' "$pid"; return; fi
  done
  return 1
}

fixture peer 30222
fixture_key peer
await_ssh peer 30222
peer_before=$(pod_identity peer)
strict_ssh peer 30222 'while true; do echo alive; sleep 1; done' > "$dir/heartbeat.log" &
heartbeat_pid=$!
for _ in {1..20}; do
  if [[ -s $dir/heartbeat.log ]]; then break; fi
  sleep 0.25
done
test -s "$dir/heartbeat.log"
fixture retired
reused_ip=$(k get pod retired -o jsonpath='{.status.podIP}')
retired_mac=$(k exec retired -- cat /sys/class/net/eth0/address)
old_k3s_pid=$(sudo systemctl show "$unit" --property=MainPID --value)
old_containerd_pid=$(containerd_pid)

cni_config() {
  jq -er '.cniconfig | if type == "string" then fromjson else . end' "$1"
}
cni_source() {
  cni_config "$1" | jq -cer '.Networks[].Config | select(.Name == "cbr0").Source | fromjson'
}
cri info > "$dir/cri-before.json"
cni_paths=$(cni_config "$dir/cri-before.json")
active=$(jq -er '.PluginConfDir + "/10-flannel.conflist"' <<< "$cni_paths")
bin_dir=$(jq -er '.PluginDirs[0]' <<< "$cni_paths")
sudo cat "$active" > "$dir/original.json"
diff -u <(jq -S . "$dir/original.json") <(cni_source "$dir/cri-before.json" | jq -S .)
jq -e '[.plugins[].type] == ["flannel","portmap","bandwidth"]' "$dir/original.json"
jq '.plugins |= [.[] | ., (select(.type == "flannel") | {type:"mwc-flannel-neighbor",bridge:"cni0"})]' \
  "$dir/original.json" > "$dir/candidate.json"
sudo install -m 0755 "$plugin" "$bin_dir/mwc-flannel-neighbor"
atomic_config() {
  local pending
  pending=$(sudo mktemp "${active%/*}/.mwc-cni-XXXXXX")
  sudo tee "$pending" < "$1" >/dev/null
  sudo chmod --reference="$active" "$pending"
  sudo mv -fT "$pending" "$active"
}
wait_cni() {
  local config=$1 file=$2
  for _ in {1..50}; do
    if cri info > "$file" && jq -e '.lastCNILoadStatus == "OK"' "$file" >/dev/null && \
      diff -q <(jq -S . "$config") <(cni_source "$file" | jq -S .) >/dev/null; then return; fi
    sleep 0.2
  done
  return 1
}

begin=$(date +%s%3N)
atomic_config "$dir/candidate.json"
wait_cni "$dir/candidate.json" "$dir/cri-candidate.json"
reload_ms=$(( $(date +%s%3N) - begin ))
k delete pod retired --wait=true --timeout=75s
lease_dir=/var/lib/cni/networks/cbr0
for _ in {1..30}; do
  if ! sudo test -e "$lease_dir/$reused_ip"; then break; fi
  sleep 0.2
done
# Advance host-local to the released address. Never edit the lease itself.
prior_ip=$(python3 -c 'import ipaddress,sys; print(ipaddress.ip_address(sys.argv[1])-1)' "$reused_ip")
sudo flock "$lease_dir/lock" bash -ec 'test ! -e "$1/$2"; printf %s "$3" > "$1/last_reserved_ip.0"' \
  _ "$lease_dir" "$reused_ip" "$prior_ip"
sudo ip neigh replace "$reused_ip" dev cni0 lladdr "$retired_mac" nud stale
sudo ip neigh get "$reused_ip" dev cni0 | grep -F "$retired_mac"
fixture replacement 30223
test "$(k get pod replacement -o jsonpath='{.status.podIP}')" = "$reused_ip"
replacement_mac=$(k exec replacement -- cat /sys/class/net/eth0/address)
test "$replacement_mac" != "$retired_mac"
if sudo ip neigh show to "$reused_ip" dev cni0 | grep -F "$retired_mac"; then
  echo 'Pod address was reused but its stale neighbor was not cleared' >&2
  exit 1
fi
fixture_key replacement
await_ssh replacement 30223
sudo ip neigh get "$reused_ip" dev cni0 | grep -F "$replacement_mac"

beats_before_rollback=$(wc -l < "$dir/heartbeat.log")
begin=$(date +%s%3N)
atomic_config "$dir/original.json"
wait_cni "$dir/original.json" "$dir/cri-rollback.json"
rollback_ms=$(( $(date +%s%3N) - begin ))
await_ssh replacement 30223
k delete pod replacement --wait=true --timeout=75s
fixture after-rollback 30224
fixture_key after-rollback
await_ssh after-rollback 30224
test "$(pod_identity peer)" = "$peer_before"
kill -0 "$heartbeat_pid"
test "$(wc -l < "$dir/heartbeat.log")" -gt "$beats_before_rollback"
await_ssh peer 30222
test "$(sudo systemctl show "$unit" --property=MainPID --value)" = "$old_k3s_pid"
test "$(containerd_pid)" = "$old_containerd_pid"

jq -n --argjson reload "$reload_ms" --argjson rollback "$rollback_ms" --arg ip "$reused_ip" \
  --argjson heartbeats "$(wc -l < "$dir/heartbeat.log")" \
  '{k3s:"v1.36.2+k3s1",containerd:"2.3.2-k3s2",loaded_ms:$reload,rollback_ms:$rollback,reused_ip:$ip,
    old_and_new_pod_cleanup:true,strict_nodeport_ssh_before_during_and_after:true,existing_ssh_heartbeats:$heartbeats,
    peer_pod_and_container_unchanged:true,k3s_and_containerd_processes_unchanged:true}' > "$dir/acceptance.json"
