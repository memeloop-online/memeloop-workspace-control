#!/usr/bin/env bash
# Only invoked in the disposable cold-boot VM with its two empty virtio disks.
set -euo pipefail
umask 077
src=/home/ubuntu/mwc-ci
state=/var/lib/mwc-workspace-reboot-ci
root=/var/lib/rancher/k3s/agent/containerd
templates=/var/lib/rancher/k3s/agent/etc/containerd
runtime=/run/k3s/containerd
kubelet=/var/lib/kubelet
namespace=memeloop-workspace-control
fixture="$src/mwc-native-buildkit-true.json"
k() { /usr/local/bin/k3s kubectl "$@"; }
w() { k -n "$namespace" "$@"; }

if [[ $1 == install ]]; then
  apt-get update >&2
  DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends lvm2 thin-provisioning-tools xfsprogs jq >&2
  install -d -m 0700 "$state" "$state/wrapper"
  install -m 0755 "$src/k3s" /usr/local/bin/k3s
  install -Dm 0755 "$src/mwc-runtime-wrapper" /usr/local/libexec/mwc-runtime-wrapper
  # The wrapper intentionally rejects symlinks on trusted paths. Resolve the
  # bundled runc's versioned path before K3s starts the integrated containerd.
  k3s ctr version --help >/dev/null
  bin=$(realpath -e /var/lib/rancher/k3s/data/current/bin)
  runc=$(realpath -e "$bin/runc")
  "$runc" --version | grep -Fx 'runc version 1.4.2' >&2
  # The guest uses --prefer-bundled-bin. Restore the original directory
  # entry on rollback, including if it is a symlink to a multicall binary.
  daemon="$bin/containerd"
  if [[ ! -x $daemon ]]; then daemon="$bin/aux/containerd"; fi
  test -x "$daemon"
  mv -T "$daemon" "$daemon.mwc-ci-original"
  install -m 0755 "$src/containerd-shim-cleanup" "$daemon"
  printf '%s\n' "$daemon" > "$state/daemon-path"
  for kind in home scratch; do
    disk="/dev/disk/by-id/virtio-mwc-ci-$kind"
    if [[ $kind == home ]]; then size=12G; else size=6G; fi
    test -b "$disk"
    pvcreate "$disk"
    vgcreate "mwc-ci-$kind" "$disk"
    lvcreate -L "$size" --poolmetadatasize 32M -T "mwc-ci-$kind/pool"
  done >&2
  vg=$(vgs --noheadings -o vg_uuid mwc-ci-scratch | tr -d '[:space:]-')
  lv=$(lvs --noheadings -o lv_uuid mwc-ci-scratch/pool | tr -d '[:space:]-')
  pool_name=$(dmsetup ls --target thin-pool | awk '$1 ~ /^mwc--ci--scratch-/ {print $1}')
  pool=$(dmsetup info -c --noheadings -o uuid "$pool_name" | tr -d '[:space:]')
  test "$pool" = "LVM-$vg$lv-tpool"
  jq -n --arg real "$runc" --arg root "$root" --arg runtime "$runtime" --arg state "$state/wrapper" --arg ns "$namespace" \
    --arg vg "$vg" --arg pool "$pool" --arg dmsetup "$(realpath -e "$(command -v dmsetup)")" '
    {real_runtime:$real,kubelet_root:"/var/lib/kubelet",
     cri_root:($root+"/io.containerd.grpc.v1.cri"),cri_state_root:($runtime+"/io.containerd.grpc.v1.cri"),
     workspace_namespace:$ns,pod_log_root:"/var/log/pods",bundle_root:($runtime+"/io.containerd.runtime.v2.task/k8s.io"),
     snapshot_root:($root+"/io.containerd.snapshotter.v1.overlayfs/snapshots"),state_root:$state,
     mount_namespace:"/proc/1/ns/mnt",scratch_vg_uuid:$vg,scratch_pool_dm_uuid:$pool,dmsetup:$dmsetup}
  ' > /etc/mwc-runtime-wrapper.json
  install -d -m 0700 "$templates"
  cat > "$templates/config-v3.toml.tmpl" <<'TOML'
{{ template "base" . }}
[plugins.'io.containerd.cri.v1.runtime'.containerd.runtimes.mwc-native]
  runtime_type = 'io.containerd.runc.v2'
[plugins.'io.containerd.cri.v1.runtime'.containerd.runtimes.mwc-native.options]
  BinaryName = '/usr/local/libexec/mwc-runtime-wrapper'
  SystemdCgroup = true
TOML
  if [[ -e /proc/sys/kernel/apparmor_restrict_unprivileged_userns ]]; then
    printf '%s\n' 'kernel.apparmor_restrict_unprivileged_userns=0' > /etc/sysctl.d/90-mwc-ci-rootless.conf
    sysctl -p /etc/sysctl.d/90-mwc-ci-rootless.conf >&2
  fi
  exit
fi

if [[ $1 == restore-daemon ]]; then
  daemon=$(<"$state/daemon-path")
  mv -fT "$daemon.mwc-ci-original" "$daemon"
  exit
fi

if [[ $1 == diagnostics ]]; then
  k get pods,pvc -A -o wide >&2 || true
  k get events -n "$namespace" --sort-by=.metadata.creationTimestamp >&2 || true
  journalctl -u k3s.service -n 80 --no-pager >&2 || true
  if [[ -f $state/wrapper/errors.log ]]; then tail -n 30 "$state/wrapper/errors.log" >&2; fi
  exit
fi

pod=$(jq -er .names.pod "$fixture")
home=$(jq -er .names.home "$fixture")
scratch=$(jq -er .names.scratch "$fixture")
pod_json() { w get pod "$pod" -o json --request-timeout=5s; }
workspace_ssh() {
  timeout "${SSH_TIMEOUT:-20}" ssh -F /dev/null -p 2222 -o BatchMode=yes -o StrictHostKeyChecking=yes -o HostKeyAlias=mwc-ci-workspace \
    -o GlobalKnownHostsFile=/dev/null -o "UserKnownHostsFile=$state/known-hosts" \
    -o IdentitiesOnly=yes -o ConnectTimeout=4 -i "$state/client" "user@$1" "${@:2}"
}

case $1 in
  seed)
    image=$(jq -er .workspace_image "$src/public-input-identity.json")
    test "$(jq -er '.pod.spec.containers[]|select(.name=="workspace").image' "$fixture")" = "$image"
    k3s ctr -n k8s.io images import --platform linux/amd64 "$src/workspace-image.tar" >&2
    k3s ctr -n k8s.io images ls -q | grep -Fx "$image" >&2
    # containerd's content import and CRI's image-event index are separate.
    # The renderer names this local image, so wait for that exact CRI tag.
    for _ in {1..30}; do
      if k3s crictl inspecti "$image" > "$state/workspace-image.json" 2>/dev/null; then break; fi
      sleep 1
    done
    test "$(jq -er .status.id "$state/workspace-image.json")" = "$(jq -er .workspace_config_digest "$src/public-input-identity.json")"
    rm "$src/workspace-image.tar"
    tar -xf "$src/helm.tgz" -C "$state"
    KUBECONFIG=/etc/rancher/k3s/k3s.yaml "$state/linux-amd64/helm" install topolvm "$src/topolvm.tgz" \
      -n topolvm-system --create-namespace -f "$src/topolvm-values.yaml" --wait --timeout 5m >&2
    printf '%s\n' '{"apiVersion":"node.k8s.io/v1","kind":"RuntimeClass","metadata":{"name":"mwc-native"},"handler":"mwc-native"}' |
      k apply -f - >&2
    ssh-keygen -q -t ed25519 -N '' -f "$state/client"
    ssh-keygen -q -t ed25519 -N '' -f "$state/host"
    printf 'mwc-ci-workspace %s\n' "$(cat "$state/host.pub")" > "$state/known-hosts"
    # Inject one stable public key. The ttyd client key lives only as long as its Pod sandbox.
    jq --rawfile key "$state/client.pub" '
      (.injections[]|select(.data["workspace-files.json"] != null).data) |=
      (.["ci-client-key"]=$key | .["workspace-files.json"] |= (fromjson |
        .files+= [{key:"ci-client-key",kind:"ssh_public_key",target:"/home/user/.ssh/authorized_keys",
        source:"workspace",object:"config_map",data_key:"ci-client-key"}] | tojson)) |
      {apiVersion:"v1",kind:"List",items:([.namespace,.scratch,.service,.config]+.injections)}
    ' "$fixture" | k apply -f - >&2
    w create secret generic "$(jq -er .identity.metadata.name "$fixture")" \
      --from-file="ssh_host_ed25519_key=$state/host" --from-file="ssh_host_ed25519_key.pub=$state/host.pub" >&2
    jq '.statefulset' "$fixture" | w apply -f - >&2
    w wait --for=create "pod/$pod" --timeout=120s >&2
    w wait --for=condition=Ready "pod/$pod" --timeout=420s >&2
    address=$(pod_json | jq -er .status.podIP)
    workspace_ssh "$address" 'set -eu
      printf home-before-reboot > /home/user/ci-home-reboot
      printf scratch-before-reboot > /var/lib/mwc/build-scratch/ci-scratch-reboot
      sync -f /home/user/ci-home-reboot
      sync -f /var/lib/mwc/build-scratch/ci-scratch-reboot' >&2
    ;;
  observe)
    previous=${2:-}
    ready=false
    for ((attempt=0; attempt<90; attempt++)); do
      doc=$(pod_json 2>/dev/null) || doc='{}'
      id=$(jq -r '.status.containerStatuses[]? | select(.name=="workspace" and .ready) | .containerID' <<< "$doc")
      address=$(jq -r '.status.podIP // empty' <<< "$doc")
      if [[ -n $id && $id != "$previous" && -n $address ]] &&
        workspace_ssh "$address" 'set -eu
          test "$(cat /home/user/ci-home-reboot)" = home-before-reboot
          test "$(cat /var/lib/mwc/build-scratch/ci-scratch-reboot)" = scratch-before-reboot
          printf writable > /home/user/.ci-reboot-writable
          printf writable > /var/lib/mwc/build-scratch/.ci-reboot-writable
          rustc --version >&2
          timeout 10 buildctl debug workers >&2' < /dev/null >&2; then
        ready=true
        break
      fi
      sleep 2
    done
    test "$ready" = true
    uid=$(jq -er .metadata.uid <<< "$doc")
    volume_rows=()
    for claim in "$home" "$scratch"; do
      pvc=$(w get "pvc/$claim" -o json)
      pv=$(k get "pv/$(jq -er .spec.volumeName <<< "$pvc")" -o json)
      pv_name=$(jq -er .metadata.name <<< "$pv")
      handle=$(jq -er '.spec.csi | select(.driver=="topolvm.io") | .volumeHandle' <<< "$pv")
      publish="$kubelet/pods/$uid/volumes/kubernetes.io~csi/$pv_name/mount"
      test "$(findmnt -n -M "$publish" -o FSTYPE)" = xfs
      lv_uid=$(lvs --noheadings -o lv_uuid --select "lv_name=$handle" | xargs)
      test -n "$lv_uid"
      volume_rows+=("$(jq -n --arg claim "$claim" --arg pvc "$(jq -er .metadata.uid <<< "$pvc")" \
        --arg pv "$(jq -er .metadata.uid <<< "$pv")" --arg handle "$handle" --arg lv "$lv_uid" \
        '{claim:$claim,pvc_uid:$pvc,pv_uid:$pv,volume_handle:$handle,lv_uuid:$lv}')")
    done
    # Prove the new container uses the real runtime and writable layers still share scratch.
    scratch_pv=$(w get "pvc/$scratch" -o jsonpath='{.spec.volumeName}')
    scratch_mount="$kubelet/pods/$uid/volumes/kubernetes.io~csi/$scratch_pv/mount"
    upper=$(jq -er --arg scratch "$scratch_mount" --arg uid "$uid" '
      select(.pod_uid==$uid and .phase!="deleted" and .scratch.path==$scratch) |
      .upper | select(startswith($scratch+"/runtime-private/"))
    ' "$state/wrapper/${id#containerd://}.json")
    test "$(stat -Lc %d "$upper")" = "$(stat -Lc %d "$scratch_mount")"
    cri_version=$(k3s crictl version | awk '/^RuntimeVersion:/ {print $2}')
    [[ $cri_version =~ ^v?2\.3\.2-k3s2-shim-cleanup$ ]]
    volumes=$(printf '%s\n' "${volume_rows[@]}" | jq -s .)
    jq -n --arg boot "$(cat /proc/sys/kernel/random/boot_id)" --arg cid "$id" --arg pod "$uid" \
      --arg version "$cri_version" --argjson volumes "$volumes" \
      '{boot_id:$boot,container_id:$cid,pod_uid:$pod,volumes:$volumes,cri_version:$version,strict_ssh:true,buildkit:true}'
    ;;
  expand)
    pod_identity() {
      pod_json | jq -ce '{uid:.metadata.uid,revision:.metadata.labels["controller-revision-hash"],
        ready:([.status.conditions[]|select(.type=="Ready")|.status]==["True"]),
        containers:(.status.containerStatuses|map({name,containerID,restartCount}))}'
    }
    before=$(pod_identity)
    test "$(jq -r .ready <<< "$before")" = true
    address=$(pod_json | jq -er .status.podIP)
    coproc GROW_SSH { SSH_TIMEOUT=170 workspace_ssh "$address" 'exec sh -s'; }
    ssh_pid=$GROW_SSH_PID
    exec {send}>&"${GROW_SSH[1]}" {recv}<&"${GROW_SSH[0]}"
    ask() {
      printf '%s\n' "$1" >&"$send"
      IFS= read -r -t 10 -u "$recv" answer
    }
    measure='df -B1 --output=size /home/user /tmp | awk "NR>1" | xargs'
    check_files='test "$(cat /home/user/ci-home-reboot)" = home-before-reboot &&
      test "$(cat /var/lib/mwc/build-scratch/ci-scratch-reboot)" = scratch-before-reboot'
    ask "$check_files && $measure"
    read -r initial_home initial_scratch <<< "$answer"
    ask 'export LC_ALL=C
      free=$(df -B1 --output=avail /tmp | tail -1)
      test "$free" -gt 134217728 && fallocate -l "$((free-16777216))" /tmp/ci-grow-fill &&
        error=$(fallocate -l 33554432 /tmp/ci-grow-write 2>&1) || :
      case "$error" in *"No space left on device"*) echo ENOSPC;; *) echo no_ENOSPC;; esac'
    test "$answer" = ENOSPC
    grow() {
      local claim=$1 role=$2 pvc uid pv size target amount patch started
      pvc=$(w get pvc "$claim" -o json)
      uid=$(jq -er .metadata.uid <<< "$pvc")
      pv=$(jq -er .spec.volumeName <<< "$pvc")
      size=$(jq -er '.spec.resources.requests.storage | select(test("^[0-9]+Gi$"))' <<< "$pvc")
      target="$((${size%Gi}+1))Gi"
      patch=$(jq -nc --arg uid "$uid" --arg source "$size" --arg target "$target" '
        [{op:"test",path:"/metadata/uid",value:$uid},
         {op:"test",path:"/spec/resources/requests/storage",value:$source},
         {op:"replace",path:"/spec/resources/requests/storage",value:$target}]')
      started=$(date +%s%3N)
      w patch pvc "$claim" --type=json -p "$patch" >&2
      for _ in {1..65}; do
        pvc=$(w get pvc "$claim" -o json --request-timeout=5s)
        jq -e --arg uid "$uid" --arg pv "$pv" '.metadata.uid==$uid and .spec.volumeName==$pv' <<< "$pvc" >/dev/null
        ask "$check_files && $measure"
        read -r measured_home measured_scratch <<< "$answer"
        if [[ $role == scratch ]]; then
          test "$measured_home" = "$initial_home"
          amount=$((measured_scratch-initial_scratch))
        else
          test "$measured_scratch" = "$grown_scratch"
          amount=$((measured_home-initial_home))
        fi
        if (( amount >= 800000000 )) &&
           jq -e --arg target "$target" '.status.capacity.storage==$target and ((.status.conditions//[])|length)==0' <<< "$pvc" >/dev/null; then
          elapsed_ms=$(($(date +%s%3N)-started))
          return
        fi
        sleep 1
      done
      return 1
    }
    grow "$scratch" scratch
    grown_scratch=$measured_scratch
    scratch_ms=$elapsed_ms
    ask 'fallocate -l 33554432 /tmp/ci-grow-write && rm /tmp/ci-grow-write /tmp/ci-grow-fill && echo writable'
    test "$answer" = writable
    grow "$home" home
    home_ms=$elapsed_ms
    ask "$check_files && printf writable > /home/user/.ci-grow-writable && echo writable"
    test "$answer" = writable
    printf '%s\n' 'exit 0' >&"$send"
    wait "$ssh_pid"
    exec {send}>&- {recv}<&-
    test "$(pod_identity)" = "$before"
    workspace_ssh "$address" "$check_files && test \"\$(cat /home/user/.ci-grow-writable)\" = writable" </dev/null
    jq -n --argjson scratch_ms "$scratch_ms" --argjson home_ms "$home_ms" '
      {same_pod_and_containers:true,continuous_strict_ssh:true,new_strict_ssh:true,
       both_claims_retain_identity:true,isolated_growth:true,write_after_ENOSPC:true,
       scratch_expand_ms:$scratch_ms,home_expand_ms:$home_ms}'
    ;;
  reclaim)
    uid=$(pod_json | jq -er .metadata.uid)
    pvs=(); handles=()
    for claim in "$home" "$scratch"; do
      pv=$(w get "pvc/$claim" -o jsonpath='{.spec.volumeName}')
      pvs+=("pv/$pv")
      handles+=("$(k get "pv/$pv" -o jsonpath='{.spec.csi.volumeHandle}')")
    done
    w delete statefulset "${pod%-0}" --timeout=120s >&2
    w wait --for=delete "pod/$pod" --timeout=180s >&2
    w delete pvc "$home" "$scratch" --timeout=120s >&2
    k wait --for=delete "${pvs[@]}" --timeout=180s >&2
    mounted=$(findmnt -rn -o TARGET)
    if grep -F "$kubelet/pods/$uid/" <<< "$mounted"; then exit 1; fi
    remaining=$(lvs --noheadings -o lv_name | xargs -n1)
    for handle in "${handles[@]}"; do
      if grep -Fxq "$handle" <<< "$remaining"; then exit 1; fi
    done
    jq -n '{node_unpublish_observed:true,pv_and_lv_reclaimed:true}'
    ;;
  *) exit 2 ;;
esac
