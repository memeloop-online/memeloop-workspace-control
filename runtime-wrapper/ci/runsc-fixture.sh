#!/bin/bash
# Actual runsc handler, grouped Sentry and per-container Gofers. No native fallback.
pod_log="$base/pod-logs/memeloop-workspace-control_mwc-budget-ci_$uid"
jq -n --arg uid "$uid" --arg log "$pod_log" '{metadata:{name:"mwc-budget-ci",namespace:"memeloop-workspace-control",uid:$uid,attempt:0},log_directory:$log,linux:{security_context:{namespace_options:{network:0}}}}' > "$base/pod.json"
mkdir -p "$pod_log"
sandbox="$(cri runp --runtime mwc-runsc "$base/pod.json")"
containers+=("$sandbox")
kubelet_hosts="$base/kubelet/pods/$uid/etc-hosts"
printf '127.0.0.1 localhost mwc-budget.local\n::1 localhost\n' > "$kubelet_hosts"
managed_host_root="$base/containerd-root/io.containerd.grpc.v1.cri/sandboxes/$sandbox"
sha256sum "$kubelet_hosts" "$managed_host_root/hostname" "$managed_host_root/resolv.conf" > "$artifacts/managed-host-before.sha256"
# SSH runs across processes inside the actual gVisor network stack. This is not
# evidence of external ingress/CNI, which remains product composition work.
cp "$base/client" "$home/ci-client"
cp "$base/known_hosts" "$home/ci-known-hosts"
chown 1000:1000 "$home/ci-client" "$home/ci-known-hosts"
chmod 0600 "$home/ci-client"
runsc_make() {
 local name="$1"; shift
 jq -n --arg name "$name" --arg home "$home" --arg data "$scratch/data" --arg hosts "$kubelet_hosts" --args '{metadata:{name:$name,attempt:0},image:{image:"docker.io/library/mwc-runtime-fixture:ci"},command:["/usr/local/bin/mwc-budget-workload"],args:$ARGS.positional,log_path:($name+"/0.log"),mounts:[{container_path:"/etc/hosts",host_path:$hosts},{container_path:"/home/user",host_path:$home},{container_path:"/scratch",host_path:$data},{container_path:"/tmp",host_path:($data+"/tmp")},{container_path:"/var/tmp",host_path:($data+"/tmp")}],linux:{security_context:{capabilities:{drop_capabilities:["SYS_ADMIN"]},seccomp:{profile_type:0},no_new_privs:($name=="ttyd"),readonly_rootfs:($name=="ttyd")}}}' "$@" > "$base/$name.json"
 local id
 id="$(cri create "$sandbox" "$base/$name.json" "$base/pod.json")"
 containers+=("$id"); last_id="$id"
 cri start "$id"
}
runsc_make init init
init_id="$last_id"
for n in $(seq 1 30); do if [ "$(cri inspect "$init_id" | jq -r .status.state)" = CONTAINER_EXITED ]; then break; fi; sleep 1; done
test "$(cri inspect "$init_id" | jq -r .status.exitCode)" = 0
runsc_make ttyd idle
ttyd_id="$last_id"
runsc_make workspace serve
main_id="$last_id"
for pair in "sandbox:$sandbox" "init:$init_id" "ttyd:$ttyd_id" "workspace:$main_id"; do
 name="${pair%%:*}"; id="${pair#*:}"
 cp "$base/state/$id.json" "$artifacts/$name-wrapper.json"
 cp "$base/pre-wrapper-$id-oci.json" "$artifacts/$name-pre-wrapper-oci.json"
 if [ -f "$base/containerd-state/io.containerd.runtime.v2.task/k8s.io/$id/config.json" ]; then cp "$base/containerd-state/io.containerd.runtime.v2.task/k8s.io/$id/config.json" "$artifacts/$name-oci.json"; fi
 if [ -f "$base/start-$id-record.json" ]; then cp "$base/start-$id-record.json" "$artifacts/$name-before-start-wrapper.json"; cp "$base/start-$id-mount.txt" "$artifacts/$name-before-start-mount.txt"; fi
done
cri inspect "$main_id" > "$artifacts/workspace-inspect.json"
cri inspect "$ttyd_id" > "$artifacts/ttyd-inspect.json"
python3 "$GITHUB_WORKSPACE/runtime-wrapper/ci/managed-files.py" "$artifacts"
cri exec "$ttyd_id" /usr/local/bin/mwc-budget-workload readonly-check > "$artifacts/readonly-sidecar-process.json"
cri exec "$main_id" bash -c 'set -e; test -f /proc/gvisor/kernel_is_gvisor; uname -a; cat /proc/version; test ! -e /scratch/runtime-private; test ! -e /runtime-private; getent hosts mwc-budget.local' > "$artifacts/application-gvisor-identity.txt"
python3 - "$base" "$artifacts" "$main_id" "$ttyd_id" "$sandbox" <<'PYX'
import json,os,pathlib,sys
base,out=map(pathlib.Path,sys.argv[1:3]);ids=sys.argv[3:]
proof=[]
for cid in ids:
 states=list((base/'runsc-state/k8s.io').glob(cid+'_sandbox:*.state'))
 assert len(states)==1,states
 state=json.loads(states[0].read_text())
 assert state['goferMountConfs'] and all(c['upper']==0 for c in state['goferMountConfs']),state['goferMountConfs']
 sentry,gofer=state['sandbox']['pid'],state['goferPid']
 assert sentry>0 and gofer>0,(sentry,gofer)
 processes=[]
 for role,pid in [('sentry',sentry),('gofer',gofer)]:
  proc=pathlib.Path('/proc',str(pid));cmd=proc.joinpath('cmdline').read_bytes().replace(b'\0',b' ').decode()
  status=dict(line.split(':',1) for line in proc.joinpath('status').read_text().splitlines() if ':' in line)
  assert status['Seccomp'].strip()=='2',(role,status['Seccomp'])
  assert os.readlink(proc/'ns/mnt')!=os.readlink('/proc/self/ns/mnt'),role
  fds=[]
  for fd in (proc/'fd').iterdir():
   try:fds.append(os.readlink(fd))
   except FileNotFoundError:pass
  assert not any('filestore' in f for f in fds),fds
  processes.append({'role':role,'pid':pid,'command':cmd,'host_seccomp':2,'fd_targets':fds})
 proof.append({'container_id':cid,'sandbox_id':state['sandbox']['id'],'sentry_pid':sentry,'gofer_pid':gofer,'gofer_mount_confs':state['goferMountConfs'],'processes':processes})
assert len({p['sentry_pid'] for p in proof})==1,proof
assert len({p['gofer_pid'] for p in proof})==3,proof
(out/'grouped-sentry-gofer-proof.json').write_text(json.dumps(proof,indent=2))
PYX
sentry_pid="$(jq -r '.[0].sentry_pid' "$artifacts/grouped-sentry-gofer-proof.json")"
# No allow-suid=true or user capability grant is introduced. Public-key SSH's
# unprivileged account writes Home directly; native sudo proof is separate.
ssh_ci() { cri exec "$main_id" ssh -i /home/user/ci-client -p 2222 -o BatchMode=yes -o StrictHostKeyChecking=yes -o UserKnownHostsFile=/home/user/ci-known-hosts -o ConnectTimeout=5 user@127.0.0.1 "$@"; }
for n in $(seq 1 30); do if ssh_ci 'id'; then break; fi; sleep 1; done
ssh_ci 'echo ssh-before-full > /home/user/ssh-before-full; echo strict-key-gvisor-ssh-home-ok' > "$artifacts/ssh-before-full.txt"
cri exec "$main_id" bash -c 'set -e; printf changed >> /seed-copy-up; dd if=/dev/zero of=/scratch/initial bs=1M count=32 conv=fsync; for name in hosts hostname resolv.conf; do printf "\n# shared-%s\n" "$name" >> "/etc/$name"; done'
cri exec "$ttyd_id" bash -c 'set -e; for name in hosts hostname resolv.conf; do grep -F "# shared-$name" "/etc/$name"; done; dd if=/dev/zero of=/scratch/sidecar bs=1M count=16 conv=fsync' > "$artifacts/managed-shared.txt"
sha256sum --check "$artifacts/managed-host-before.sha256" > "$artifacts/managed-host-after-append.txt"
cri exec "$main_id" /usr/local/bin/mwc-budget-workload runsc-fill > "$artifacts/runsc-full-result.json"
kill -0 "$sentry_pid"
cri inspect "$main_id" > "$artifacts/main-after-full.json"
test "$(jq -r .status.state "$artifacts/main-after-full.json")" = CONTAINER_RUNNING
ssh_ci 'echo home-survives-full > /home/user/after-full; echo strict-key-gvisor-ssh-home-after-full' > "$artifacts/ssh-after-full.txt"
sha256sum --check "$artifacts/managed-host-before.sha256" > "$artifacts/managed-host-after-full.txt"
cri exec "$main_id" bash -c 'set -e; rm /rootfs-fill /scratch/data-fill /scratch/initial /scratch/sidecar; echo recovered > /rootfs-after-free; echo recovered > /scratch/recovered; sync'
# A readonly child delete removes only its own upper, preserving the live main
# overlay, shared managed files, logs, pause root and grouped Sentry.
cri stop "$ttyd_id"; cri rm "$ttyd_id"
test "$(jq -r .phase "$base/state/$ttyd_id.json")" = deleted
test ! -e "$scratch/runtime-private/$ttyd_id"
test -d "$scratch/runtime-private/$main_id/upper"
test -d "$scratch/runtime-private/$sandbox/upper"
kill -0 "$sentry_pid"
cri exec "$main_id" bash -c 'set -e; echo brother-alive > /rootfs-after-child-delete; echo brother-alive > /scratch/after-child-delete'
ssh_ci 'echo strict-key-gvisor-ssh-after-child-delete' > "$artifacts/ssh-after-child-delete.txt"
"$base/mwc-runsc-wrapper" --root="$base/runsc-state/k8s.io" delete --force "$ttyd_id"
# The exact four override families pass through CRI's explicit test annotation
# projection and must fail before any private upper/rootfs redirect is created.
for prefix in dev.gvisor.spec.rootfs. dev.gvisor.spec.mount. dev.gvisor.flag. dev.gvisor.bundle.; do
 name="$(echo "$prefix" | tr . -)"
 jq --arg name "$name" --arg key "${prefix}override" '.metadata.name=$name|.annotations={($key):"test-override"}' "$base/workspace.json" > "$base/reject-hint.json"
 id="$(cri create "$sandbox" "$base/reject-hint.json" "$base/pod.json")"; containers+=("$id")
 if cri start "$id" > "$artifacts/reject-$name.txt" 2>&1; then echo 'runsc hint bypass unexpectedly started'; exit 1; fi
 grep -F "unsupported runsc override annotation" "$base/state/errors.log"
 test ! -e "$scratch/runtime-private/$id"
 cri rm "$id"
done
# Actual shim conflicting TOML must fail before rootfs mutation, with no native
# replacement. This is a separate sandbox, not a child in the accepted group.
conflict_log="$base/pod-logs/memeloop-workspace-control_mwc-conflict_$uid"
jq --arg log "$conflict_log" '.metadata.name="mwc-conflict"|.log_directory=$log' "$base/pod.json" > "$base/conflict-pod.json"
mkdir "$conflict_log"
if cri runp --runtime mwc-runsc-conflict "$base/conflict-pod.json" > "$artifacts/reject-conflict.txt" 2>&1; then echo 'runsc conflict unexpectedly accepted'; exit 1; fi
grep -F 'runsc overlay2 must be none' "$base/state/errors.log"
# Invalid child executable tests real start failure and shim's normal delete.
jq '.metadata.name="bad-executable"|.command=["/not-an-executable"]|.args=[]' "$base/workspace.json" > "$base/bad.json"
bad_id="$(cri create "$sandbox" "$base/bad.json" "$base/pod.json")";containers+=("$bad_id")
if cri start "$bad_id" > "$artifacts/bad-start.txt" 2>&1; then echo 'invalid executable unexpectedly started'; exit 1; fi
cri rm "$bad_id"
test "$(jq -r .phase "$base/state/$bad_id.json")" = deleted
kill -0 "$sentry_pid"
# Actual CRI stdout logs share the already verified filesystem. Fill this path,
# then retain Sentry/SSH/Home health; no kubelet rotation claim is inferred.
cri exec "$main_id" python3 -c 'f=open("/proc/1/fd/1","wb",buffering=0);[f.write(b"L"*8191+b"\n") for _ in range(80000)]' > "$artifacts/log-burst-exec.txt" 2>&1
python3 - "$pod_log/workspace/0.log" "$scratch" "$artifacts" <<'PYX'
import errno,json,pathlib,sys
p,scratch,out=map(pathlib.Path,sys.argv[1:]);assert p.stat().st_dev==scratch.stat().st_dev
count=0
try:
 with p.open('ab',buffering=0) as f:
  while True:count+=f.write(b'x'*1048576)
except OSError as e:
 assert e.errno in (errno.ENOSPC,errno.EDQUOT),e
 (out/'log-enospc.json').write_text(json.dumps({'errno':e.errno,'additional_append_bytes':count,'same_scratch_device':True}))
PYX
kill -0 "$sentry_pid"
ssh_ci 'echo logs-full-home-alive > /home/user/logs-full; echo strict-key-gvisor-ssh-logs-full-home-alive' > "$artifacts/ssh-after-logs-full.txt"
cri inspect "$main_id" > "$artifacts/main-after-logs-full.json"
test "$(jq -r .status.state "$artifacts/main-after-logs-full.json")" = CONTAINER_RUNNING
# Stop/delete leaves no Gofer or Sentry reference; keep only Pod-scoped temporary
# logs/managed files for the existing scratch owner, not one child's cleanup.
for id in "$main_id" "$init_id"; do
 cri stop "$id"; cri rm "$id"
 test "$(jq -r .phase "$base/state/$id.json")" = deleted
 test ! -e "$scratch/runtime-private/$id"
done
cri stopp "$sandbox"; cri rmp "$sandbox"
test "$(jq -r .phase "$base/state/$sandbox.json")" = deleted
! kill -0 "$sentry_pid" 2>/dev/null
! grep -F "upperdir=$scratch/runtime-private/" /proc/self/mountinfo
python3 - "$artifacts/grouped-sentry-gofer-proof.json" <<'PYX'
import json,pathlib,sys
for c in json.load(open(sys.argv[1])):
 assert not pathlib.Path('/proc',str(c['gofer_pid'])).exists(),c
PYX
umount "$scratch"; scratch_detached=1
umount "$home"; home_detached=1
printf '%s\n' 'actual grouped runsc, Gofer no-overlay, rootfs/data/etc ENOSPC, readonly sidecar, independent Home/strict SSH, mmap outcome with Sentry alive, child-only and final normal cleanup passed' > "$artifacts/runsc-acceptance.txt"
