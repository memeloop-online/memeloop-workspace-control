#!/bin/bash
# Separate disposable VM/mount namespace, retaining the full functional jobs.
if [ "$mode" = runsc ]; then handler=mwc-runsc; network=0; wrapper="$base/mwc-runsc-wrapper"; else handler=mwc-ci; network=2; wrapper="$base/mwc-runtime-wrapper"; fi
control_log="$base/pod-logs/memeloop-workspace-control_mwc-hardkill-control_$uid"
mkdir -p "$control_log"
jq -n --arg uid "$uid" --arg log "$control_log" --argjson network "$network" '{metadata:{name:"mwc-hardkill-control",namespace:"memeloop-workspace-control",uid:$uid,attempt:0},log_directory:$log,linux:{security_context:{namespace_options:{network:$network}}}}' > "$base/control-pod.json"
sandbox="$(cri runp --runtime "$handler" "$base/control-pod.json")"
containers+=("$sandbox")
hosts="$base/kubelet/pods/$uid/etc-hosts"
printf '127.0.0.1 localhost mwc-budget.local\n::1 localhost\n' > "$hosts"
jq -n --arg home "$home" --arg data "$scratch/data" --arg hosts "$hosts" '{metadata:{name:"control",attempt:0},image:{image:"docker.io/library/mwc-runtime-fixture:ci"},command:["/usr/local/bin/mwc-budget-workload","idle"],log_path:"control/0.log",mounts:[{container_path:"/etc/hosts",host_path:$hosts},{container_path:"/home/user",host_path:$home},{container_path:"/scratch",host_path:$data}],linux:{security_context:{capabilities:{drop_capabilities:["SYS_ADMIN"]},seccomp:{profile_type:0}}}}' > "$base/control.json"
control_id="$(cri create "$sandbox" "$base/control.json" "$base/control-pod.json")"
containers+=("$control_id")
cri start "$control_id"
control_alive() {
 cri exec "$control_id" bash -c 'set -e; echo control > /control-root; echo control > /scratch/control; echo control > /home/user/control; test -f /control-root'
 test -d "$scratch/runtime-private/$control_id/upper"
 test -d "$scratch/runtime-private/$sandbox/upper"
}
control_alive
blocked=0
for stage in prepared redirected runtime-created; do
 out="$artifacts/hardkill-$stage"
 log="$base/pod-logs/memeloop-workspace-control_mwc-kill-$stage"_"$uid"
 mkdir -p "$out" "$log"
 jq --arg stage "$stage" --arg log "$log" '.metadata.name=("mwc-kill-"+$stage)|.log_directory=$log' "$base/control-pod.json" > "$base/kill-pod.json"
 if cri runp --runtime "mwc-kill-$stage" "$base/kill-pod.json" > "$out/create-rpc.txt" 2>&1; then echo "hardkill unexpectedly returned success: $stage"; exit 1; fi
 test -f "$out/observed.json"
 id="$(cat "$out/id.txt")"
 cri pods -o json > "$out/after-rpc-cri-sandboxes.json"
 cri ps -a -o json > "$out/after-rpc-cri-containers.json"
 ctr_ci tasks list > "$out/after-rpc-tasks.txt"
 cat /proc/self/mountinfo > "$out/after-rpc-mountinfo.txt"
 cp "$base/state/$id.json" "$out/after-rpc-record.json"
 if [ -d "$base/containerd-state/io.containerd.runtime.v2.task/k8s.io/$id" ]; then
  for name in log.json config.json init.pid; do
   path="$base/containerd-state/io.containerd.runtime.v2.task/k8s.io/$id/$name"
   [ ! -f "$path" ] || cp "$path" "$out/after-rpc-$name"
  done
 fi
 # Ask only the actual failed sandbox ID to stop/delete. If CRI no longer has
 # it, still invoke the real wrapper's normal single-ID delete; absence itself
 # grants no permission to remove files or unmount anything in this fixture.
 cri stopp "$id" > "$out/normal-cri-stop.txt" 2>&1 || true
 cri rmp "$id" > "$out/normal-cri-delete.txt" 2>&1 || true
 root="$(cat "$out/runtime-root.txt")"
 if "$wrapper" --root "$root" delete --force "$id" > "$out/normal-wrapper-delete.txt" 2>&1; then delete_status=0; else delete_status=$?; blocked=1; fi
 cat /proc/self/mountinfo > "$out/after-delete-mountinfo.txt"
 cp "$base/state/$id.json" "$out/after-delete-record.json"
 python3 "$GITHUB_WORKSPACE/runtime-wrapper/ci/hardkill-references.py" "$out" after-delete
 python3 - "$out" "$scratch" "$id" "$delete_status" <<'PYX'
import json,pathlib,sys
out,scratch=map(pathlib.Path,sys.argv[1:3]);cid=sys.argv[3];status=int(sys.argv[4])
record=json.loads((out/'after-delete-record.json').read_text())
mounts=(out/'after-delete-mountinfo.txt').read_text().splitlines()
result={'normal_delete_status':status,'record_phase':record['phase'],'upper_directory_exists':(scratch/'runtime-private'/cid).exists(),'mount_references':[m for m in mounts if cid in m],'real_csi':False,'crash_recovery_implemented':False}
(out/'result.json').write_text(json.dumps(result,indent=2))
PYX
 if ! jq -e '.normal_delete_status==0 and .record_phase=="deleted" and .upper_directory_exists==false and (.mount_references|length)==0' "$out/result.json"; then blocked=1; fi
 control_alive
done
cri stop "$control_id"; cri rm "$control_id"
cri stopp "$sandbox"; cri rmp "$sandbox"
for id in "$control_id" "$sandbox"; do test "$(jq -r .phase "$base/state/$id.json")" = deleted; done
cat /proc/self/mountinfo > "$artifacts/hardkill-final-mountinfo.txt"
if umount "$scratch" > "$artifacts/hardkill-normal-scratch-unmount.txt" 2>&1; then scratch_detached=1; else blocked=1; fi
if umount "$home" > "$artifacts/hardkill-normal-home-unmount.txt" 2>&1; then home_detached=1; else blocked=1; fi
cp -a "$base/state" "$artifacts/hardkill-final-wrapper-state"
test "$blocked" = 0
printf '%s\n' 'three externally observed wrapper SIGKILL edges, live control preserved, real normal ID delete and ordinary CSI-shaped scratch/Home unmount passed; no real CSI/reboot/crash GC acceptance' > "$artifacts/hardkill-acceptance.txt"
