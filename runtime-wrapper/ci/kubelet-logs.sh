#!/bin/bash
# Sourced only by the disposable privileged fixture. Real kubelet owns log
# reading/rotation/reopen/previous/summary/GC; no simulated du/GC acceptance.
set -euo pipefail
live_kubelet_root="$base/kubelet-live"
mkdir -p "$live_kubelet_root"
jq --arg root "$live_kubelet_root" ' .kubelet_root=$root' /etc/mwc-runtime-wrapper.json > "$base/logs-runtime-config.json"
install -m 0600 "$base/logs-runtime-config.json" /etc/mwc-runtime-wrapper.json
jq -n --arg logs "$base/pod-logs" '{apiVersion:"kubelet.config.k8s.io/v1beta1",kind:"KubeletConfiguration",podLogsDir:$logs,containerLogMaxSize:"1Mi",containerLogMaxFiles:3,containerLogMonitorInterval:"15s",cgroupDriver:"cgroupfs"}' > "$base/kubelet-logs.conf"
K3S_DATA_DIR="$base/k3s-control" "$base/k3s-bin" server \
 --data-dir "$base/k3s-control" --write-kubeconfig "$base/kubeconfig" \
 --node-name mwc-ci --container-runtime-endpoint "unix://$base/containerd.sock" \
 --flannel-backend none --disable-network-policy --disable-kube-proxy \
 --disable coredns,traefik,servicelb,local-storage,metrics-server \
 --disable-helm-controller --disable-cloud-controller \
 --kubelet-arg "root-dir=$live_kubelet_root" --kubelet-arg "config=$base/kubelet-logs.conf" \
 > "$base/k3s-control.log" 2>&1 &
k3s_pid=$!
kube() { KUBECONFIG="$base/kubeconfig" "$base/k3s-bin" kubectl "$@"; }
for n in $(seq 1 120); do
 if kube get node mwc-ci > "$artifacts/kubelet-node.txt" 2>/dev/null; then break; fi
 if ! kill -0 "$k3s_pid"; then tail -n 30 "$base/k3s-control.log"; exit 1; fi
 sleep 1
done
kube create namespace memeloop-workspace-control
kube create serviceaccount -n memeloop-workspace-control mwc-ci
kube apply -f - <<'YAML'
apiVersion: node.k8s.io/v1
kind: RuntimeClass
metadata:
  name: mwc-ci
handler: mwc-ci
YAML
# schedulingGates keeps kubelet away until the API-assigned UID has a trusted
# fixture publish root. This is CI plumbing, not a production resolver bypass.
jq -n --arg home "$home" --arg data "$scratch/data" '{apiVersion:"v1",kind:"Pod",metadata:{name:"mwc-log-ci",namespace:"memeloop-workspace-control"},spec:{serviceAccountName:"mwc-ci",automountServiceAccountToken:false,runtimeClassName:"mwc-ci",hostNetwork:true,dnsPolicy:"Default",schedulingGates:[{name:"mwc-ci/publish-ready"}],containers:[{name:"workspace",image:"docker.io/library/mwc-runtime-fixture:ci",imagePullPolicy:"Never",command:["/usr/local/bin/mwc-budget-workload","serve"],volumeMounts:[{name:"home",mountPath:"/home/user"},{name:"scratch",mountPath:"/scratch"}]},{name:"terminal",image:"docker.io/library/mwc-runtime-fixture:ci",imagePullPolicy:"Never",tty:true,command:["/bin/bash","-c","echo tty-stdout-start; echo tty-stderr-start >&2; exec sleep infinity"]}],volumes:[{name:"home",hostPath:{path:$home,type:"Directory"}},{name:"scratch",hostPath:{path:$data,type:"Directory"}}]}}' > "$base/kubelet-pod.json"
kube apply -f "$base/kubelet-pod.json"
kube_uid="$(kube get pod -n memeloop-workspace-control mwc-log-ci -o jsonpath='{.metadata.uid}')"
kubelet_publish="$live_kubelet_root/pods/$kube_uid/volumes/kubernetes.io~csi/$scratch_pv/mount"
mkdir -p "$kubelet_publish"
mount --bind "$scratch" "$kubelet_publish"
cp "$(dirname "$scratch")/vol_data.json" "$(dirname "$kubelet_publish")/vol_data.json"
kube patch pod -n memeloop-workspace-control mwc-log-ci --type json -p '[{"op":"remove","path":"/spec/schedulingGates"}]'
kube wait -n memeloop-workspace-control --for=condition=Ready pod/mwc-log-ci --timeout=150s
kube_log="$base/pod-logs/memeloop-workspace-control_mwc-log-ci_$kube_uid"
test -L "$kube_log"
test "$(readlink "$kube_log")" = "$kubelet_publish/runtime-private/logs/$kube_uid"
work_id="$(kube get pod -n memeloop-workspace-control mwc-log-ci -o json | jq -r '.status.containerStatuses[]|select(.name=="workspace").containerID|sub("containerd://";"")')"
kube logs -n memeloop-workspace-control mwc-log-ci -c workspace > "$artifacts/kubectl-workspace-logs.txt"
kube logs -n memeloop-workspace-control mwc-log-ci -c terminal > "$artifacts/kubectl-tty-logs.txt"
grep -F workspace-stdout-start "$artifacts/kubectl-workspace-logs.txt"
grep -F workspace-stderr-start "$artifacts/kubectl-workspace-logs.txt"
grep -F tty-stdout-start "$artifacts/kubectl-tty-logs.txt"
grep -F tty-stderr-start "$artifacts/kubectl-tty-logs.txt"
kube get --raw /api/v1/nodes/mwc-ci/proxy/stats/summary > "$artifacts/summary-before-logs.json"
cri exec "$work_id" python3 -c 'import os; f=open("/proc/1/fd/1","wb",buffering=0);f.write(b"stats-growth-line\n"*32768)'
for n in $(seq 1 25); do
 kube get --raw /api/v1/nodes/mwc-ci/proxy/stats/summary > "$artifacts/summary-after-logs.json"
 if python3 - "$kube_uid" "$artifacts" <<'PYX'
import json,pathlib,sys
uid,out=sys.argv[1],pathlib.Path(sys.argv[2])
def pod(name):return next(p for p in json.loads((out/name).read_text())['pods'] if p['podRef']['uid']==uid)
a,b=pod('summary-before-logs.json'),pod('summary-after-logs.json')
used=lambda p:sum(c.get('logs',{}).get('usedBytes',0) for c in p['containers'])
assert used(b)>used(a),(used(a),used(b))
assert b['ephemeral-storage']['usedBytes']>=used(b),b
(out/'actual-log-stats.json').write_text(json.dumps({'before_log_used_bytes':used(a),'after_log_used_bytes':used(b),'pod_ephemeral_used_bytes':b['ephemeral-storage']['usedBytes'],'logs_capacity_reported_as_node_not_scratch':True,'eviction_settings_unchanged':True},indent=2))
PYX
 then break; fi
 sleep 1
done
test -f "$artifacts/actual-log-stats.json"
# The real kubelet rotates, requests CRI ReopenContainerLog, and compresses old
# rotations. Follow uses the real API streaming/read path across the rename.
kube logs -f --tail=0 -n memeloop-workspace-control mwc-log-ci -c workspace > "$artifacts/kubectl-follow-rotation.txt" 2>&1 &
follow_pid=$!
sleep 1
cri exec "$work_id" python3 -c 'f=open("/proc/1/fd/1","wb",buffering=0);f.write(b"follow-before-rotation\n");f.write(b"rotation-line\n"*180000)'
for n in $(seq 1 45); do
 if find "$kube_log/workspace" -name '0.log.*' -print -quit | grep .; then break; fi
 sleep 1
done
cri exec "$work_id" python3 -c 'f=open("/proc/1/fd/1","wb",buffering=0);f.write(b"follow-after-rotation\n");f.write(b"second-rotation-line\n"*100000)'
for n in $(seq 1 45); do
 if find "$kube_log/workspace" -name '*.gz' -print -quit | grep .; then break; fi
 sleep 1
done
find "$kube_log/workspace" -name '*.gz' > "$artifacts/real-kubelet-compressed-rotations.txt"
test -s "$artifacts/real-kubelet-compressed-rotations.txt"
grep -F follow-before-rotation "$artifacts/kubectl-follow-rotation.txt"
grep -F follow-after-rotation "$artifacts/kubectl-follow-rotation.txt"
kill "$follow_pid"; wait "$follow_pid" || true
# Restart only the container in the active Pod; kubelet must expose --previous.
cri exec "$work_id" python3 -c 'open("/proc/1/fd/1","wb",buffering=0).write(b"previous-container-marker\n")'
for n in $(seq 1 10); do
 if kube logs -n memeloop-workspace-control mwc-log-ci -c workspace | grep -F previous-container-marker; then break; fi
 sleep .2
done
cri stop "$work_id"
for n in $(seq 1 90); do
 next_id="$(kube get pod -n memeloop-workspace-control mwc-log-ci -o json | jq -r '([.status.containerStatuses[]|select(.name=="workspace" and .state.running!=null)|.containerID][0] // "")|sub("containerd://";"")')"
 if [ "$next_id" != "$work_id" ] && [ "${#next_id}" = 64 ]; then break; fi
 sleep 1
done
kube wait -n memeloop-workspace-control --for=condition=Ready pod/mwc-log-ci --timeout=90s
kube logs --previous -n memeloop-workspace-control mwc-log-ci -c workspace > "$artifacts/kubectl-previous-logs.txt"
grep -F previous-container-marker "$artifacts/kubectl-previous-logs.txt"
test -L "$kube_log"
work_id="$next_id"
printf '%s\n' 'owned-log-target-marker' > "$scratch/runtime-private/logs/$kube_uid/ci-gc-marker"
# Fill via the actual container stdout/CRI logger within one monitor interval.
# Rotation later can free capacity; no node-disk fallback is allowed meanwhile.
cri exec "$work_id" python3 -c 'import os;f=open("/proc/1/fd/1","wb",buffering=0);[f.write(b"L"*8191+b"\n") for _ in range(80000)];open("/home/user/log-burst-done","w").write("done")' > "$artifacts/log-burst-exec.txt" 2>&1 &
burst_pid=$!
for n in $(seq 1 100); do
 if [ "$(df --output=avail -B1 "$scratch" | tail -1 | xargs)" -lt 1048576 ]; then break; fi
 sleep .1
done
python3 - "$kube_log/workspace" "$artifacts" <<'PYX'
import errno,json,pathlib,sys
root,out=map(pathlib.Path,sys.argv[1:]);p=max(root.glob('*.log'),key=lambda p:p.stat().st_mtime)
count=0
try:
 with p.open('ab',buffering=0) as f:
  while True:count+=f.write(b'z'*1048576)
except OSError as e:
 assert e.errno in (errno.ENOSPC,errno.EDQUOT),e
 (out/'log-capacity-error.json').write_text(json.dumps({'errno':e.errno,'additional_log_append_bytes':count,'actual_cri_log':str(p.resolve())},indent=2))
PYX
ssh_ci 'sudo -n sh -c "echo log-full-ssh-alive > /home/user/log-full-ssh"; echo strict-ssh-log-full-home-writable' > "$artifacts/ssh-after-logs-full.txt"
cri inspect "$work_id" > "$artifacts/workspace-after-logs-full.json"
test "$(jq -r .status.state "$artifacts/workspace-after-logs-full.json")" = CONTAINER_RUNNING
wait "$burst_pid"
test -f "$home/log-burst-done"
# Ended-Pod logs may end with scratch, per product semantics. The real kubelet
# must unlink its top-level symlink during normal Pod GC, not recurse into the
# target; the fixture then owns clearing its temporary filesystem.
kube delete pod -n memeloop-workspace-control mwc-log-ci --wait=false
for n in $(seq 1 90); do
 if [ "$(cri ps --label "io.kubernetes.pod.uid=$kube_uid" -o json | jq '.containers|length')" = 0 ] && [ "$(cri pods --state Ready --label "io.kubernetes.pod.uid=$kube_uid" -o json | jq '.items|length')" = 0 ] && ! grep -F "upperdir=$kubelet_publish/runtime-private/" /proc/self/mountinfo; then break; fi
 sleep 1
done
test "$(cri ps --label "io.kubernetes.pod.uid=$kube_uid" -o json | jq '.containers|length')" = 0
test "$(cri pods --state Ready --label "io.kubernetes.pod.uid=$kube_uid" -o json | jq '.items|length')" = 0
! grep -F "upperdir=$kubelet_publish/runtime-private/" /proc/self/mountinfo
# This fixture, not an installed CSI driver, owns this synthetic publish. Follow
# CSI ordering: stop tasks, ordinary Unpublish, then allow kubelet's normal GC.
umount "$kubelet_publish"
publish_parent="$(dirname "$kubelet_publish")"
rmdir "$kubelet_publish"
rm "$publish_parent/vol_data.json"
rmdir "$publish_parent"
kubelet_publish=''
printf '%s\n' 'normal CSI-like publish unmount after stopped CRI tasks passed; no log bind mount' > "$artifacts/log-publish-unmount.txt"
kube wait -n memeloop-workspace-control --for=delete pod/mwc-log-ci --timeout=90s
for n in $(seq 1 150); do
 if [ ! -e "$kube_log" ] && [ ! -L "$kube_log" ]; then break; fi
 sleep 1
done
test ! -L "$kube_log"
test ! -e "$kube_log"
test "$(cat "$scratch/runtime-private/logs/$kube_uid/ci-gc-marker")" = owned-log-target-marker
printf '%s\n' 'real kubelet Pod GC unlinked only the Pod log symlink; scratch target retained for owning lifecycle' > "$artifacts/real-kubelet-log-gc.txt"
! grep -F " $kube_log " /proc/self/mountinfo
kill "$k3s_pid"; wait "$k3s_pid" || true
k3s_pid=''
