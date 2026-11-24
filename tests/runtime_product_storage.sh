#!/bin/bash
# Only an expendable CI VM. No synthetic CSI publication is permitted.
set -euo pipefail
: "${GITHUB_ACTIONS:?Disposable CI VM only}"
inputs="${MWC_PRODUCT_STORAGE_INPUTS:?Verified public CSI inputs required}"
copy_input() {
 local name="$1" digest="$2" destination="$3"
 cp "$inputs/$name" "$destination"
 printf '%s  %s\n' "$digest" "$destination" | sha256sum -c -
}
: "${RUNNER_TEMP:?}"
: "${GITHUB_WORKSPACE:?}"
base=/var/lib/mwc-product-storage-ci
containerd_state="$base/containerd-state"
bundle_root="$containerd_state/io.containerd.runtime.v2.task/k8s.io"
pod_log_root=/var/log/pods
out="$GITHUB_WORKSPACE/artifacts"
fixtures="$RUNNER_TEMP/product-storage-fixtures"
namespace=memeloop-workspace-control
mkdir -p "$base" "$out" "$base/state/mwc-native" "$base/state/mwc-runsc"
chmod 0700 "$base/state" "$base/state/"*
exec > >(tee "$out/product-csi.log") 2>&1
# The whole harness, both daemons, their shims, and the wrapper share this
# namespace. Keep the shell alive through cleanup as its stable identity anchor.
mount_namespace="/proc/$$/ns/mnt"
test "$(readlink "$mount_namespace")" != "$(readlink /proc/1/ns/mnt)"
test "$(findmnt -n -o PROPAGATION --mountpoint /)" = private
# Give this disposable namespace a shared root and state parent like a node.
# The initial recursive-private unshare keeps these changes out of the runner.
mount --make-shared /
install -d -o root -g root -m 0711 "$bundle_root"
mount --bind "$containerd_state" "$containerd_state"
mount --make-shared "$containerd_state"
test -z "$(find "$bundle_root" -mindepth 1 -print -quit)"
if findmnt -n --mountpoint "$bundle_root" >/dev/null; then exit 1; fi
mount --bind "$bundle_root" "$bundle_root"
mount --make-private "$bundle_root"
# Model the canonical path on a node with group-writable /var/log without
# touching the runner's log directory or changing the kubelet's log location.
install -d -o root -g "$(id -g nobody)" -m 0775 "$base/node-logs"
install -d -o root -g root -m 0750 "$base/node-logs/pods"
mount --bind "$base/node-logs" /var/log
log_inode="$(stat -c %d:%i "$pod_log_root")"
printf '%s\n' original-log-directory > "$pod_log_root/existing-marker"
mount --bind "$pod_log_root" "$pod_log_root"
test "$(stat -c %d:%i "$pod_log_root")" = "$log_inode"
runuser -u nobody -- python3 - <<'PY'
import errno
import os

os.mkdir('/var/log/rename-probe')  # Confirm the parent really is group-writable.
try:
    os.rename('/var/log/pods', '/var/log/rename-probe/pods')
except OSError as error:
    if error.errno != errno.EBUSY:
        raise
else:
    raise RuntimeError('canonical log mount was renamed')
os.rmdir('/var/log/rename-probe')
PY
k3s_pid=''; containerd_pid=''; loops=(); diagnostic_case=''; diagnostic_monitor_pid=''
runtime_config=''; restore_config=''
kube() { KUBECONFIG="$base/kubeconfig" "$base/k3s-bin" kubectl "$@"; }
cri() { crictl --runtime-endpoint "unix://$base/containerd.sock" --image-endpoint "unix://$base/containerd.sock" "$@"; }
ctr_ci() { ctr --address "$base/containerd.sock" --namespace k8s.io "$@"; }
restore_wrapper_config() {
 if [ -n "$restore_config" ]; then
  mv "$restore_config" "$runtime_config" || return 1
  restore_config=''
 fi
}
writeback_state() {
 python3 "$GITHUB_WORKSPACE/tests/runtime_product_storage_bdi.py" "$vg_uuid" "$pool_uuid" "$@"
}
check_active_writeback() {
 local checkpoint="$1" scratch_path="$2" home_path="$3"
 writeback_state "$scratch_path" "$home_path" > "$case_out/writeback-$checkpoint.json"
 jq -es '
  .[0] as $before | .[1] as $after |
  ($before.scratch.knobs.strict_limit==0) and
  ($after.scratch.knobs.strict_limit==1) and
  ([$before.scratch.dm_uuid, ($before.scratch.knobs | del(.strict_limit))] ==
   [$after.scratch.dm_uuid, ($after.scratch.knobs | del(.strict_limit))]) and
  ([$before.home.dm_uuid, $before.home.knobs] == [$after.home.dm_uuid, $after.home.knobs]) and
  ($before.scope==$after.scope)
 ' "$case_out/writeback-rejected.json" "$case_out/writeback-$checkpoint.json"
}
# Preserve only storage/process identities, never the runsc state's OCI env.
# Frozen cgroup paths remain observable after an OOM has removed its PIDs.
storage_diagnostics() {
 local native_pid=''
 if [ "$1" = before-write ] && [ "$runtime" = mwc-native ]; then
  native_pid="$(cri inspect "$main_id" | jq -er --arg id "$main_id" 'select(.status.id==$id) | .info.pid | select(type=="number" and .>0)')"
 fi
 python3 - "$base" "$diagnostic_case" "$runtime" "$main_id" "$publish" "$1" "$native_pid" <<'PY'
import datetime, gzip, json, os, pathlib, re, stat, subprocess, sys
base, output = map(pathlib.Path, sys.argv[1:3])
runtime, main, publish, stage, native_pid = sys.argv[3:]
identity_file = output / 'runtime-storage-identity.json'
result = {'stage': stage, 'time': datetime.datetime.now(datetime.timezone.utc).isoformat()}
def read_bounded(path):
    try:
        with pathlib.Path(path).open() as stream:
            value = stream.read(65537)
        return {'available': True, 'text': value[:65536], 'truncated': len(value) > 65536}
    except OSError as error:
        return {'available': False, 'error': str(error)}
try:
    if stage == 'before-write':
        record = json.loads((base / 'state' / runtime / (main + '.json')).read_text())
        upper = pathlib.Path(publish) / 'runtime-private' / main / 'upper'
        assert record['id'] == main and record['phase'] == 'created'
        assert record['scratch']['path'] == publish and record['upper'] == str(upper)
        mounts = [line for line in pathlib.Path('/proc/self/mountinfo').read_text().splitlines()
                  if line.split()[4] == record['rootfs']]
        assert len(mounts) == 1 and ' - overlay ' in mounts[0], mounts
        assert 'upperdir=' + str(upper) in mounts[0].split(' - ', 1)[1].split()[2].split(',')
        identity = {'main_id': main, 'publish': publish, 'upper': str(upper),
                    'scratch_mount': record['scratch']['mount'], 'scratch_inode': record['scratch']['inode'],
                    'scratch_dm_uuid': record['scratch']['dm_uuid'],
                    'rootfs_mount': mounts[0],
                    'processes': [], 'gofer_mounts': [], 'cgroups': []}
        cgroups = set()
        def process_group(pid):
            proc = pathlib.Path('/proc', str(pid))
            membership = proc.joinpath('cgroup').read_text().splitlines()
            unified = [line[3:] for line in membership if line.startswith('0::')]
            assert len(unified) == 1 and unified[0].startswith('/'), membership
            group = pathlib.Path('/sys/fs/cgroup') / unified[0].lstrip('/')
            assert group.is_relative_to('/sys/fs/cgroup') and '..' not in group.parts
            assert (group / 'memory.events').is_file() and (group / 'memory.current').is_file(), group
            parent = group
            while True:
                cgroups.add(str(parent))
                if parent == pathlib.Path('/sys/fs/cgroup'):
                    break
                parent = parent.parent
            return proc, group
        if runtime == 'mwc-native':
            pid = int(native_pid)
            assert pid > 0
            proc, group = process_group(pid)
            assert main in group.name, (main, str(group))
            # This is the recreated workspace, not the pre-log-restart CRI PID.
            start = proc.joinpath('stat').read_text().rsplit(') ', 1)[1].split()[19]
            identity['processes'].append({'role': 'native-main', 'pid': pid,
                                          'starttime_ticks': int(start), 'cgroup': str(group)})
        if runtime == 'mwc-runsc':
            pod = json.loads((output / 'pod-current.json').read_text())
            ids = [s['containerID'].removeprefix('containerd://') for s in pod['status']['containerStatuses']]
            states = []
            for cid in ids:
                paths = list((base / 'runsc-state/k8s.io').glob(cid + '_sandbox:*.state'))
                assert len(paths) == 1, (cid, paths)
                states.append((cid, json.loads(paths[0].read_text())))
            sandbox_ids = {state['sandbox']['id'] for _, state in states}
            assert len(sandbox_ids) == 1, sandbox_ids
            sandbox_id = sandbox_ids.pop()
            paths = list((base / 'runsc-state/k8s.io').glob(sandbox_id + '_sandbox:*.state'))
            assert len(paths) == 1, paths
            states.append((sandbox_id, json.loads(paths[0].read_text())))
            processes = set()
            for cid, state in states:
                confs = state['goferMountConfs']
                assert confs and all(c['upper'] == 0 for c in confs), (cid, confs)
                identity['gofer_mounts'].append({'container_id': cid, 'mount_confs': confs})
                for role, pid in [('sentry', state['sandbox']['pid']), ('gofer', state['goferPid'])]:
                    assert pid > 0, (role, pid)
                    if (role, pid) in processes:
                        continue
                    processes.add((role, pid))
                    proc, group = process_group(pid)
                    fd_targets = []
                    for fd in proc.joinpath('fd').iterdir():
                        try:
                            fd_targets.append(os.readlink(fd))
                        except FileNotFoundError:
                            pass
                    assert not any('filestore' in target for target in fd_targets)
                    status = proc.joinpath('status').read_text().splitlines()
                    seccomp = [line.split(':', 1)[1].strip() for line in status if line.startswith('Seccomp:')]
                    assert seccomp == ['2'], (role, pid, seccomp)
                    identity['processes'].append({'role': role, 'pid': pid, 'cgroup': str(group),
                                                  'host_seccomp': 2, 'filestore_fd_present': False})
        identity['cgroups'] = sorted(cgroups)
        identity['cgroup_inodes'] = {group: pathlib.Path(group).stat().st_ino for group in identity['cgroups']}
        identity_file.write_text(json.dumps(identity, indent=2) + '\n')
    identity = json.loads(identity_file.read_text())
    result['memory'] = []
    for group in identity['cgroups']:
        entry = {'path': group}
        try:
            entry['inode'] = pathlib.Path(group).stat().st_ino
            assert entry['inode'] == identity['cgroup_inodes'][group], ('cgroup replaced', group)
        except OSError as error:
            entry['inode_error'] = str(error)
        for name in ('memory.current', 'memory.max', 'memory.high', 'memory.peak', 'memory.events',
                     'memory.events.local', 'memory.stat', 'memory.pressure', 'cgroup.procs',
                     'cgroup.controllers', 'cgroup.subtree_control', 'cgroup.type', 'io.stat'):
            try:
                entry[name] = pathlib.Path(group, name).read_text()
            except OSError as error:
                entry[name] = {'error': str(error)}
        result['memory'].append(entry)
    # Read existing kernel interfaces only; no sysctl, tracepoint or cgroup writes.
    config_keys = ('CGROUPS', 'MEMCG', 'BLK_CGROUP', 'CGROUP_WRITEBACK', 'DEBUG_FS', 'XFS_FS')
    result['kernel_config'] = []
    for path in (pathlib.Path('/proc/config.gz'), pathlib.Path('/boot/config-' + os.uname().release)):
        try:
            text = gzip.open(path, 'rt').read() if path.suffix == '.gz' else path.read_text()
            selected = [line for line in text.splitlines()
                        if any(line.startswith('CONFIG_' + key + '=') or
                               line == '# CONFIG_' + key + ' is not set' for key in config_keys)]
            result['kernel_config'].append({'path': str(path), 'available': True, 'selected': selected})
        except OSError as error:
            result['kernel_config'].append({'path': str(path), 'available': False, 'error': str(error)})
    # The harness already proved that this disposable VM has a private mount
    # namespace. Expose kernel writeback counters there without altering an
    # existing debugfs mount or enabling tracepoints. Preserve CONFIG on failure.
    debug_mounts = [line for line in pathlib.Path('/proc/self/mountinfo').read_text().splitlines()
                    if line.split()[4] == '/sys/kernel/debug' and ' - debugfs ' in line]
    if stage == 'before-write' and not debug_mounts:
        mounted = subprocess.run(['mount', '-t', 'debugfs', '-o', 'ro', 'debugfs', '/sys/kernel/debug'],
                                 capture_output=True, text=True)
        result['debugfs_mount_attempt'] = {'status': mounted.returncode, 'stderr': mounted.stderr[:4096]}
        assert mounted.returncode == 0, 'required read-only CI debugfs mount failed; see CONFIG and mount receipt'
        debug_mounts = [line for line in pathlib.Path('/proc/self/mountinfo').read_text().splitlines()
                        if line.split()[4] == '/sys/kernel/debug' and ' - debugfs ' in line]
        assert len(debug_mounts) == 1 and 'ro' in debug_mounts[0].split()[5].split(','), debug_mounts
    result['debugfs_mounts'] = debug_mounts
    result['dirty_sysctls'] = {path.name: read_bounded(path)
                              for path in sorted(pathlib.Path('/proc/sys/vm').glob('dirty_*'))}
    result['vmstat'] = [line for line in pathlib.Path('/proc/vmstat').read_text().splitlines()
                        if re.match(r'nr_(dirty|writeback|dirtied|written|dirty_threshold|dirty_background_threshold) ', line)]
    result['controller_mounts'] = [line for line in pathlib.Path('/proc/self/mountinfo').read_text().splitlines()
                                   if ' - cgroup' in line]
    result['proc_cgroups'] = [line for line in pathlib.Path('/proc/cgroups').read_text().splitlines()
                              if line.startswith('#') or line.split()[0] in ('memory', 'blkio')]
    device = identity['scratch_mount']['device']
    bdi_link = pathlib.Path('/sys/dev/block', device, 'bdi')
    bdi = bdi_link.resolve(strict=False)
    result['scratch_bdi'] = {'device': device, 'sysfs_link': str(bdi_link),
                              'resolved_path': str(bdi), 'available': bdi_link.is_dir(),
                              'sysfs': {}, 'debugfs': {}}
    for name in ('read_ahead_kb', 'min_ratio', 'max_ratio', 'min_bytes', 'max_bytes', 'strict_limit',
                 'stable_pages_required'):
        result['scratch_bdi']['sysfs'][name] = read_bounded(bdi / name)
    for name in ('stats', 'wb_stats'):
        result['scratch_bdi']['debugfs'][name] = read_bounded(pathlib.Path('/sys/kernel/debug/bdi', bdi.name, name))
    if stage == 'before-write':
        assert bdi_link.is_dir() and all(item['available'] for item in result['scratch_bdi']['debugfs'].values()), \
            'required actual scratch BDI writeback counters unavailable; see diagnostic receipt'
    if stage in ('root-written', 'root-full'):
        # No-follow every component, then the exact upper file. Host fsync and
        # allocated blocks prove that guest writes reached the genuine CSI LV.
        fds = []
        try:
            for path in (identity['publish'], identity['upper']):
                fd = os.open('/', os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC)
                fds.append(fd)
                for part in pathlib.Path(path).parts[1:]:
                    fd = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC, dir_fd=fd)
                    fds.append(fd)
                if path == identity['publish']:
                    publish_fd = fd
                    published = os.fstat(fd)
                    assert published.st_ino == identity['scratch_inode']
                else:
                    upper_fd = fd
            fd = os.open('storage-ci-root-fill', os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC, dir_fd=upper_fd)
            fds.append(fd)
            before = os.fstat(fd)
            assert stat.S_ISREG(before.st_mode)
            assert before.st_dev == published.st_dev == os.fstat(upper_fd).st_dev
            assert f'{os.major(before.st_dev)}:{os.minor(before.st_dev)}' == identity['scratch_mount']['device']
            for held in (publish_fd, upper_fd, fd):
                info = pathlib.Path('/proc/self/fdinfo', str(held)).read_text().splitlines()
                mount_ids = [int(line.split(':', 1)[1]) for line in info if line.startswith('mnt_id:')]
                assert mount_ids == [identity['scratch_mount']['id']], mount_ids
            expected = 2415919104
            if stage == 'root-full':
                expected += json.loads((output / 'root-enospc.json').read_text())['written_bytes']
            assert before.st_size == expected and before.st_blocks * 512 >= expected, (before.st_size, before.st_blocks, expected)
            os.fsync(fd)
            after = os.fstat(fd)
            assert (after.st_dev, after.st_ino, after.st_size) == (before.st_dev, before.st_ino, before.st_size)
            result['host_upper_file'] = {'path': str(pathlib.Path(identity['upper']) / 'storage-ci-root-fill'),
                                         'device': identity['scratch_mount']['device'], 'inode': after.st_ino,
                                         'size': after.st_size, 'allocated_bytes': after.st_blocks * 512,
                                         'host_fsync': True}
        finally:
            for fd in reversed(fds):
                os.close(fd)
    if stage == 'finish':
        kernel = subprocess.run(['dmesg', '--time-format', 'iso'], capture_output=True, text=True)
        kernel_lines = kernel.stdout.splitlines()
        result['kernel_oom'] = [line for line in kernel_lines
                                if re.search(r'oom|out of memory|killed process|memory cgroup', line, re.I)][-80:]
        starts = [i for i, line in enumerate(kernel_lines) if 'invoked oom-killer' in line]
        if starts:
            result['kernel_oom_context'] = kernel_lines[max(0, starts[0] - 3):starts[0] + 160]
        result['kernel_read_status'] = kernel.returncode
except Exception as error:
    result['error'] = str(error)
    raise
finally:
    (output / ('runtime-storage-' + stage + '.json')).write_text(json.dumps(result, indent=2) + '\n')
PY
}
# Observe only frozen host cgroup counters while the unchanged guest writer runs.
# The sampler is bounded by the same 180-second writer deadline and owns no mounts.
storage_write() {
 local write_status=0
 python3 - "$diagnostic_case" "$1" "$base" "$case_id" "$address" <<'PY' &
import datetime, json, pathlib, signal, subprocess, sys, threading, time
output, phase = pathlib.Path(sys.argv[1]), sys.argv[2]
identity = json.loads((output / 'runtime-storage-identity.json').read_text())
base, case_id, address = sys.argv[3:]
stopping = threading.Event()
stop_requested = False
def stop(*unused):
    global stop_requested
    stop_requested = True
signal.signal(signal.SIGTERM, stop)
keys = {'anon', 'file', 'kernel', 'shmem', 'file_dirty', 'file_writeback',
        'inactive_file', 'active_file', 'pgscan', 'pgsteal', 'pgfault', 'pgmajfault'}
deadline = time.monotonic() + 180
def probe_ssh():
    command = ['ssh', '-F', '/dev/null', '-p', '2222', '-o', 'BatchMode=yes',
               '-o', 'StrictHostKeyChecking=yes', '-o', 'ConnectTimeout=5',
               '-o', 'UserKnownHostsFile=' + base + '/known-' + case_id,
               '-i', base + '/client-' + case_id, 'user@' + address, 'true']
    with (output / ('runtime-storage-' + phase + '-ssh.jsonl')).open('w') as stream:
        while not stopping.is_set() and time.monotonic() < deadline:
            started = time.monotonic()
            result = {'at': datetime.datetime.now(datetime.timezone.utc).isoformat(), 'deadline_seconds': 5}
            try:
                result['exit'] = subprocess.run(command, timeout=5, stdout=subprocess.DEVNULL,
                                                stderr=subprocess.DEVNULL).returncode
            except subprocess.TimeoutExpired:
                result['timeout'] = True
            result['seconds'] = time.monotonic() - started
            stream.write(json.dumps(result) + '\n'); stream.flush()
            stopping.wait(2)
ssh_thread = threading.Thread(target=probe_ssh)
ssh_thread.start()
with (output / ('runtime-storage-' + phase + '-pressure.jsonl')).open('w') as stream:
    while True:
        if stop_requested or time.monotonic() >= deadline:
            break
        sample = {'time': datetime.datetime.now(datetime.timezone.utc).isoformat(), 'memory': []}
        for group in identity['cgroups']:
            entry = {'path': group}
            try:
                entry['inode'] = pathlib.Path(group).stat().st_ino
                if entry['inode'] != identity['cgroup_inodes'][group]:
                    entry['error'] = 'frozen cgroup inode changed; counters not read'
                    sample['memory'].append(entry)
                    continue
            except OSError as error:
                entry['inode_error'] = str(error)
            for name in ('memory.current', 'memory.events', 'memory.events.local', 'memory.pressure', 'io.pressure', 'memory.stat'):
                try:
                    text = pathlib.Path(group, name).read_text()
                    entry[name] = ({line.split()[0]: int(line.split()[1]) for line in text.splitlines()
                                    if line.split()[0] in keys} if name == 'memory.stat' else text)
                except OSError as error:
                    entry[name] = {'available': False, 'error': str(error)}
            sample['memory'].append(entry)
        bdi = pathlib.Path('/sys/dev/block', identity['scratch_mount']['device'], 'bdi').resolve(strict=False)
        sample['scratch_bdi_limits'] = {}
        for name in ('max_bytes', 'max_ratio_fine', 'strict_limit'):
            try:
                sample['scratch_bdi_limits'][name] = (bdi / name).read_text().strip()
            except OSError as error:
                sample['scratch_bdi_limits'][name] = {'available': False, 'error': str(error)}
        try:
            with (pathlib.Path('/sys/kernel/debug/bdi', bdi.name, 'wb_stats')).open() as stats:
                text = stats.read(65537)
            sample['scratch_wb_stats'] = {'available': True, 'text': text[:65536], 'truncated': len(text) > 65536}
        except OSError as error:
            sample['scratch_wb_stats'] = {'available': False, 'error': str(error)}
        stream.write(json.dumps(sample) + '\n')
        stream.flush()
        stopping.wait(min(0.5, max(0, deadline - time.monotonic())))
stopping.set()
ssh_thread.join(timeout=6)
PY
 diagnostic_monitor_pid=$!
 local started=$SECONDS
 timeout 180s "$base/k3s-bin" kubectl --kubeconfig "$base/kubeconfig" exec -n "$namespace" "$pod" -c workspace -- /usr/local/bin/storage-ci-probe "$@" || write_status=$?
 local elapsed=$((SECONDS-started))
 kill "$diagnostic_monitor_pid" 2>/dev/null || true
 wait "$diagnostic_monitor_pid" || write_status=1
 diagnostic_monitor_pid=''
 jq -n --arg phase "$1" --argjson seconds "$elapsed" --argjson status "$write_status" \
  '{phase:$phase,elapsed_seconds:$seconds,exit:$status}' > "$diagnostic_case/runtime-storage-$1-duration.json"
 return "$write_status"
}
snapshot() {
 kube get pods,pvc -A -o json > "$out/final-pods-pvc.json" 2>/dev/null || true
 kube get pv,logicalvolumes.topolvm.io -o json > "$out/final-pv-lv.json" 2>/dev/null || true
 kube get events -A -o json > "$out/events.json" 2>/dev/null || true
 kube logs -n topolvm-system -l app.kubernetes.io/component=node --all-containers --prefix > "$out/topolvm-node.log" 2>&1 || true
 kube logs -n topolvm-system -l app.kubernetes.io/component=controller --all-containers --prefix > "$out/topolvm-controller.log" 2>&1 || true
 kube logs -n topolvm-system -l app.kubernetes.io/component=lvmd --all-containers --prefix > "$out/topolvm-lvmd.log" 2>&1 || true
 cat /proc/self/mountinfo > "$out/final-mountinfo.txt"
 lvs -a -o lv_name,vg_name,lv_uuid,lv_attr,pool_lv,data_percent,metadata_percent --reportformat json > "$out/final-lvm.json" || true
 if [ -d "$base/state" ]; then cp -a "$base/state" "$out/wrapper-state"; fi
}
finish() {
 status=$?
 trap - EXIT
 restore_wrapper_config || status=1
 if [ -n "$diagnostic_monitor_pid" ]; then
  kill "$diagnostic_monitor_pid" 2>/dev/null || true
  wait "$diagnostic_monitor_pid" || true
 fi
 if [ -n "$diagnostic_case" ]; then storage_diagnostics finish || status=1; fi
 snapshot
 if [ -n "$k3s_pid" ]; then kill "$k3s_pid" 2>/dev/null || true; wait "$k3s_pid" || true; fi
 if [ -n "$containerd_pid" ]; then kill "$containerd_pid" 2>/dev/null || true; wait "$containerd_pid" || true; fi
 # Never detach a mounted or busy LV to turn cleanup failure into a pass.
 for vg in mwc-ci-home mwc-ci-scratch; do
  if vgs "$vg" >/dev/null 2>&1; then
   if ! vgremove -y "$vg" >> "$out/cleanup.log" 2>&1; then status=1; fi
  fi
 done
 for loop in "${loops[@]}"; do
  if ! losetup -d "$loop" >> "$out/cleanup.log" 2>&1; then status=1; fi
 done
 jq -n --argjson exit_status "$status" '{exit_status:$exit_status,normal_cleanup:($exit_status==0)}' > "$out/cleanup-receipt.json"
 if [ "$status" != 0 ] && [ -f "$out/acceptance.json" ]; then
  jq '.accepted=false | .cleanup_failed=true' "$out/acceptance.json" > "$out/acceptance-updated.json"
  mv "$out/acceptance-updated.json" "$out/acceptance.json"
 fi
 exit "$status"
}

# GitHub prepares public, checksum-verified inputs on this disposable VM.
(cd "$inputs" && sha256sum -c SHA256SUMS)
cp "$inputs/registry/"*.json "$out/"
trap finish EXIT

copy_input k3s \
 65a55ec56c24eab44383086166ec620a491952b7e23941a49ddca6e8a4c4b4de "$base/k3s-bin"
chmod 0755 "$base/k3s-bin"
K3S_DATA_DIR="$base/k3s" "$base/k3s-bin" ctr version --help >/dev/null
export PATH="$base/k3s/data/current/bin:$PATH"
containerd --version | tee "$out/containerd-version.txt"
runc --version | tee "$out/runc-version.txt"
grep -F 'v2.3.2-k3s2' "$out/containerd-version.txt"
grep -Fx 'runc version 1.4.2' "$out/runc-version.txt"
uname -a > "$out/kernel.txt"
install -m 0755 "$RUNNER_TEMP/product-storage-bin/mwc-runtime-wrapper" "$base/mwc-runtime-wrapper"
install -m 0755 "$RUNNER_TEMP/product-storage-bin/mwc-runsc-wrapper" "$base/mwc-runsc-wrapper"
mkdir -p "$base/gvisor" "$base/runsc-state"
copy_input gvisor.tar.bz2 \
 1298e8ed9c3fee900f00d2ea96ec2d79d80ed93f2175812f29b7e7da718041a3 "$base/gvisor.tar.bz2"
tar -xjf "$base/gvisor.tar.bz2" -C "$base/gvisor"
export PATH="$base/gvisor:$PATH"
"$base/gvisor/runsc" --version | tee "$out/runsc-version.txt"
grep -F release-20261019.0 "$out/runsc-version.txt"
test -x "$base/gvisor/containerd-shim-runsc-v1"
test -d "$base/gvisor/gvisor-bin"
find "$base/gvisor" -type f -printf '%P\n' > "$out/gvisor-payload-files.txt"
for kind in home scratch; do
 if [ "$kind" = home ]; then backing=14G; pool=12G; else backing=7G; pool=6G; fi
 truncate -s "$backing" "$base/$kind.img"
 # Model a block device without an extra buffered backing-file cache. The
 # kernel may reject or disable DIO, so verify effective flags before LVM I/O.
 if ! loop="$(losetup --find --show --direct-io=on "$base/$kind.img")"; then
  echo "Required direct I/O loop setup failed for $kind" >&2
  exit 1
 fi
 loops+=("$loop")
 losetup --list --json --output NAME,MAJ:MIN,BACK-FILE,DIO,LOG-SEC "$loop" > "$out/loop-$kind.json"
 cat "/sys/block/${loop##*/}/loop/dio" > "$out/loop-$kind-sysfs-dio.txt"
 cat "/sys/block/${loop##*/}/loop/backing_file" > "$out/loop-$kind-sysfs-backing-file.txt"
 if ! jq -e --arg device "$loop" --arg backing "$base/$kind.img" \
  '.loopdevices | length==1 and .[0].name==$device and .[0]["back-file"]==$backing and .[0].dio==true' "$out/loop-$kind.json" || \
  [ "$(cat "$out/loop-$kind-sysfs-dio.txt")" != 1 ] || \
  [ "$(cat "$out/loop-$kind-sysfs-backing-file.txt")" != "$base/$kind.img" ]; then
  echo "Required direct I/O/backing-file identity did not read back for $loop ($kind)" >&2
  exit 1
 fi
 pvcreate -y "$loop"
 vgcreate "mwc-ci-$kind" "$loop"
 lvcreate -L "$pool" --poolmetadatasize 32M -T "mwc-ci-$kind/pool"
 pvs --reportformat json -o pv_name,pv_uuid,vg_name,pv_size "$loop" > "$out/loop-$kind-pv.json"
 lvs --reportformat json -a -o lv_name,vg_name,lv_uuid,lv_attr,segtype,pool_lv,devices "mwc-ci-$kind" > "$out/loop-$kind-lvs.json"
 lsblk --json --paths --output NAME,MAJ:MIN,TYPE,SIZE,FSTYPE,PKNAME "$loop" > "$out/loop-$kind-device-stack.json"
done
vg_uuid="$(vgs --noheadings -o vg_uuid mwc-ci-scratch | tr -d '[:space:]-')"
# LVM versions do not all name an active thin pool with a -tpool suffix.
# Resolve this exact scratch LV by LVM UUID and verify the actual DM target.
pool_lv_uuid="$(lvs --noheadings -o lv_uuid mwc-ci-scratch/pool | tr -d '[:space:]-')"
pool_prefix="LVM-${vg_uuid}${pool_lv_uuid}"
dmsetup info -c --noheadings --separator : -o name,uuid > "$out/initial-dm-info.txt"
dmsetup table --target thin-pool > "$out/initial-thin-pool-tables.txt"
mapfile -t pool_uuids < <(
 while IFS=: read -r name uuid; do
  name="$(printf '%s' "$name" | xargs)"
  uuid="$(printf '%s' "$uuid" | tr -d '[:space:]')"
  case "$uuid" in
   "$pool_prefix"|"$pool_prefix"-*)
    if dmsetup table "$name" | awk '$3=="thin-pool" {found=1} END {exit !found}'; then
     printf '%s\n' "$uuid"
    fi
    ;;
  esac
 done < "$out/initial-dm-info.txt"
)
test "${#pool_uuids[@]}" = 1
pool_uuid="${pool_uuids[0]}"
jq -n --arg runtime "$(readlink -f "$(command -v runc)")" --arg vg "$vg_uuid" --arg pool "$pool_uuid" --arg dmsetup "$(readlink -f "$(command -v dmsetup)")" --arg base "$base" --arg logs "$pod_log_root" --arg namespace "$mount_namespace" \
 '{real_runtime:$runtime,kubelet_root:"/var/lib/kubelet",cri_root:($base+"/containerd-root/io.containerd.grpc.v1.cri"),cri_state_root:($base+"/containerd-state/io.containerd.grpc.v1.cri"),workspace_namespace:"memeloop-workspace-control",pod_log_root:$logs,bundle_root:($base+"/containerd-state/io.containerd.runtime.v2.task/k8s.io"),snapshot_root:($base+"/containerd-root/io.containerd.snapshotter.v1.overlayfs/snapshots"),state_root:($base+"/state/mwc-native"),mount_namespace:$namespace,scratch_vg_uuid:$vg,scratch_pool_dm_uuid:$pool,dmsetup:$dmsetup}' > /etc/mwc-runtime-wrapper.json
chmod 0600 /etc/mwc-runtime-wrapper.json
jq --arg runtime "$base/gvisor/runsc" --arg state "$base/state/mwc-runsc" '.real_runtime=$runtime | .state_root=$state' /etc/mwc-runtime-wrapper.json > /etc/mwc-runsc-wrapper.json
chmod 0600 /etc/mwc-runsc-wrapper.json
cat > "$base/runsc.toml" <<TOML
binary_name = '$base/mwc-runsc-wrapper'
root = '$base/runsc-state'
grouping = true
[runsc_config]
  systemd-cgroup = 'true'
  platform = 'systrap'
  overlay2 = 'none'
  file-access = 'shared'
  file-access-mounts = 'shared'
  oci-seccomp = 'true'
  gvisor-marker-file = 'true'
TOML
cp "$base/runsc.toml" "$out/runsc-shim-config.toml"
cat > "$base/containerd.toml" <<TOML
version = 3
root = '$base/containerd-root'
state = '$containerd_state'
[grpc]
  address = '$base/containerd.sock'
[plugins.'io.containerd.cri.v1.images']
  snapshotter = 'overlayfs'
  [plugins.'io.containerd.cri.v1.images'.pinned_images]
    sandbox = 'registry.k8s.io/pause:3.10'
[plugins.'io.containerd.cri.v1.runtime'.cni]
  bin_dirs = ['$base/k3s/data/current/bin']
  conf_dir = '$base/k3s/agent/etc/cni/net.d'
[plugins.'io.containerd.cri.v1.runtime'.containerd]
  default_runtime_name = 'runc'
  [plugins.'io.containerd.cri.v1.runtime'.containerd.runtimes.runc]
    runtime_type = 'io.containerd.runc.v2'
    [plugins.'io.containerd.cri.v1.runtime'.containerd.runtimes.runc.options]
      SystemdCgroup = true
  [plugins.'io.containerd.cri.v1.runtime'.containerd.runtimes.mwc-native]
    runtime_type = 'io.containerd.runc.v2'
    [plugins.'io.containerd.cri.v1.runtime'.containerd.runtimes.mwc-native.options]
      BinaryName = '$base/mwc-runtime-wrapper'
      SystemdCgroup = true
  [plugins.'io.containerd.cri.v1.runtime'.containerd.runtimes.mwc-runsc]
    runtime_type = 'io.containerd.runsc.v1'
    [plugins.'io.containerd.cri.v1.runtime'.containerd.runtimes.mwc-runsc.options]
      ConfigPath = '$base/runsc.toml'
TOML
containerd --config "$base/containerd.toml" > "$out/containerd.log" 2>&1 & containerd_pid=$!
for n in $(seq 1 60); do if cri info > "$out/cri-info.json" 2>/dev/null; then break; fi; sleep 1; done
cri info > "$out/cri-info.json"
# Imports and kubelet use one real containerd content store.
identity="$inputs/registry/public-input-identity.json"
jq -e --arg sha "$PRODUCT_RENDERER_SHA" '.mode=="github-public" and .source_sha==$sha and .harbor_required==false and .production_rust_d507_tested==false' "$identity"
workspace_image="$(jq -er .workspace_image "$identity")"
test "$(jq -r '.pod.spec.containers[]|select(.name=="workspace")|.image' "$fixtures/mwc-native-buildkit-false.json")" = "$workspace_image"
# Unpack the local workspace image before measuring the remaining capacity too.
ctr_ci images import "$inputs/workspace-image.tar"
for n in $(seq 1 30); do
 if cri inspecti "$workspace_image" > "$out/public-workspace-image.json" 2>/dev/null; then break; fi
 sleep 1
done
jq -e --arg config "$(jq -er .workspace_config_digest "$identity")" '.status.id==$config' "$out/public-workspace-image.json"
rm "$inputs/workspace-image.tar"
for image in \
 ghcr.io/memeloop-online/memeloop-workspace-control-ttyd@sha256:cc408d2cb9c3fe457b206cffae4a15264ae15f037eac0b1f4dde86058d499012 \
 docker.io/moby/buildkit:v0.33.0-rootless@sha256:80b15f0735e87bab7bf59ec4d695dfb4a7cfb25521cf56dc75d6f256285b63ef \
 ghcr.io/topolvm/topolvm-with-sidecar@sha256:70548dbe0c6addcccf79a557f29e95db2e6dc2cba2102988c91f30086004d0fc \
 registry.k8s.io/pause:3.10@sha256:ee6521f290b2168b6e0935a181d4cff9be1ac3f505666ef0e3c98fae8199917a \
 docker.io/rancher/mirrored-coredns-coredns:1.14.4@sha256:3e98f280fd601b37411c5fb7075fd9f337833c480f1644970b727ae0af067782; do
 # containerd v2.3.2-k3s2 images pull always unpacks; it has no skip-unpack flag.
 ctr_ci images pull --platform linux/amd64 "$image"
 # k3s requests these two built-ins by their original tags.
 case "$image" in
  registry.k8s.io/pause:*|docker.io/rancher/mirrored-coredns-coredns:*) ctr_ci images tag "$image" "${image%@*}" ;;
 esac
done
ctr_ci images ls > "$out/public-containerd-images.txt"
# Do not start the real pressure cases after input staging has consumed the
# reviewed disk reserve. The original 30 GB admission gate remains mandatory.
df -B1 "$base" > "$out/capacity-after-inputs.txt"
test "$(df -B1 --output=avail "$base" | tail -n1)" -ge 30000000000
# Keep actual production limits and security contexts. Ubuntu's VM policy must
# permit the existing unprivileged RootlessKit user namespace, as main CI does.
if [ -e /proc/sys/kernel/apparmor_restrict_unprivileged_userns ]; then sysctl -w kernel.apparmor_restrict_unprivileged_userns=0; fi
stat -c '%u:%g %a %n' / /var /var/log "$pod_log_root" > "$out/pod-log-root-permissions.txt"
K3S_DATA_DIR="$base/k3s" "$base/k3s-bin" server --data-dir "$base/k3s" --write-kubeconfig "$base/kubeconfig" \
 --node-name mwc-product-ci --container-runtime-endpoint "unix://$base/containerd.sock" \
 --disable traefik,servicelb,local-storage,metrics-server --disable-helm-controller \
 > "$out/k3s.log" 2>&1 & k3s_pid=$!
for n in $(seq 1 150); do
 if kube get node mwc-product-ci >/dev/null 2>&1; then break; fi
 if ! kill -0 "$k3s_pid"; then tail -n 40 "$out/k3s.log"; exit 1; fi
 sleep 1
done
kube wait --for=condition=Ready node/mwc-product-ci --timeout=120s
jq -n --arg host "$(readlink /proc/1/ns/mnt)" --arg harness "$(readlink "$mount_namespace")" \
 --arg containerd "$(readlink "/proc/$containerd_pid/ns/mnt")" --arg k3s "$(readlink "/proc/$k3s_pid/ns/mnt")" \
 --arg configured "$(jq -r .mount_namespace /etc/mwc-runtime-wrapper.json)" \
 '{host:$host,harness:$harness,containerd:$containerd,k3s:$k3s,wrapper_mount_namespace:$configured}' > "$out/actual-mount-namespaces.json"
jq -e --arg namespace "$mount_namespace" '.host!=.harness and .harness==.containerd and .harness==.k3s and .wrapper_mount_namespace==$namespace' "$out/actual-mount-namespaces.json"
findmnt -J -o TARGET,PROPAGATION --mountpoint /var/lib/kubelet > "$out/actual-kubelet-mount-propagation.json"
test "$(findmnt -n -o PROPAGATION --mountpoint /var/lib/kubelet)" = shared
for dir in / "$containerd_state" "$bundle_root"; do
 findmnt -n -o TARGET,PROPAGATION --mountpoint "$dir"
done > "$out/actual-bundle-mount-propagation.txt"
test "$(findmnt -n -o PROPAGATION --mountpoint /)" = shared
test "$(findmnt -n -o PROPAGATION --mountpoint "$containerd_state")" = shared
test "$(findmnt -n -o PROPAGATION --mountpoint "$bundle_root")" = private
kube get --raw /api/v1/nodes/mwc-product-ci/proxy/configz | jq '.kubeletconfig | {podLogsDir,evictionHard,evictionMinimumReclaim,containerLogMaxSize,containerLogMaxFiles}' > "$out/actual-kubelet-log-config.json"
jq -e --arg logs "$pod_log_root" '.podLogsDir==$logs' "$out/actual-kubelet-log-config.json"
kube apply -f - <<'YAML'
apiVersion: node.k8s.io/v1
kind: RuntimeClass
metadata: {name: mwc-native}
handler: mwc-native
---
apiVersion: node.k8s.io/v1
kind: RuntimeClass
metadata: {name: mwc-runsc}
handler: mwc-runsc
YAML
copy_input helm.tgz \
 c306b46f719b0a4da32d0f78ee21bf90ce8d602f15b22ab753f0674d1670a7f3 "$base/helm.tgz"
tar -xzf "$base/helm.tgz" -C "$base"
copy_input topolvm.tgz \
 4b6c070da0b38f098c9ef6b6a38ef4e2dfff52adecedc280ad92bfab408f90ee "$base/topolvm.tgz"
cat > "$base/topolvm-values.yaml" <<'YAML'
cert-manager: {enabled: false}
webhook:
  certManager: false
  podMutatingWebhook: {enabled: false}
scheduler: {enabled: false}
controller: {replicaCount: 1}
image:
  repository: ghcr.io/topolvm/topolvm-with-sidecar
  reference: 0.41.1@sha256:70548dbe0c6addcccf79a557f29e95db2e6dc2cba2102988c91f30086004d0fc
lvmd:
  deviceClasses:
    - {name: mwc-ci-home, volume-group: mwc-ci-home, type: thin, spare-gb: 0, thin-pool: {name: pool, overprovision-ratio: 2.0}}
    - {name: mwc-ci-scratch, volume-group: mwc-ci-scratch, type: thin, default: true, spare-gb: 0, thin-pool: {name: pool, overprovision-ratio: 2.0}}
storageClasses:
  - name: mwc-ci-home
    storageClass: {fsType: xfs, reclaimPolicy: Delete, volumeBindingMode: WaitForFirstConsumer, additionalParameters: {topolvm.io/device-class: mwc-ci-home}}
  - name: mwc-ci-scratch
    storageClass: {fsType: xfs, reclaimPolicy: Delete, volumeBindingMode: WaitForFirstConsumer, additionalParameters: {topolvm.io/device-class: mwc-ci-scratch}}
YAML
KUBECONFIG="$base/kubeconfig" "$base/linux-amd64/helm" install topolvm "$base/topolvm.tgz" -n topolvm-system --create-namespace -f "$base/topolvm-values.yaml" --wait --timeout 5m
kube get storageclasses,csidrivers,csinodes -o json > "$out/actual-csi-installation.json"

# An ordinary, unwrapped workload must survive the whole CSI matrix in the
# same containerd. Its own overlay and pod logs still use node storage.
kube apply -f - <<YAML
apiVersion: v1
kind: Pod
metadata: {name: mwc-default-runtime}
spec:
  automountServiceAccountToken: false
  containers:
    - name: sentinel
      image: $workspace_image
      imagePullPolicy: Never
      command: ["sh", "-ec", "printf ordinary-runtime > /tmp/mwc-sentinel; exec sleep 3600"]
YAML
kube wait --for=condition=Ready pod/mwc-default-runtime --timeout=120s
default_id="$(kube get pod mwc-default-runtime -o json | jq -er 'select(.spec.runtimeClassName==null) | .status.containerStatuses[0].containerID | sub("^containerd://"; "") | select(test("^[a-f0-9]{64}$"))')"
default_uid="$(kube get pod mwc-default-runtime -o jsonpath='{.metadata.uid}')"
default_rootfs="$bundle_root/$default_id/rootfs"
default_logs="$pod_log_root/default_mwc-default-runtime_$default_uid"
findmnt -n -o TARGET,FSTYPE,PROPAGATION --mountpoint "$default_rootfs" > "$out/default-runtime-rootfs.txt"
test "$(findmnt -n -o FSTYPE,PROPAGATION --mountpoint "$default_rootfs" | xargs)" = 'overlay private'
test -d "$default_logs" && test ! -L "$default_logs"
test "$(kube exec mwc-default-runtime -- cat /tmp/mwc-sentinel)" = ordinary-runtime

for runtime in mwc-native mwc-runsc; do
for enabled in false true; do
 case_id="$runtime-buildkit-$enabled"
 fixture="$fixtures/$case_id.json"
 case_out="$out/$case_id"; mkdir -p "$case_out"
 jq -e --arg runtime "$runtime" --argjson enabled "$enabled" '.pod.spec.runtimeClassName==$runtime and .scope.runtime==$runtime and .scope.buildkit==$enabled and .scope.scratch_gib>=4 and .scratch.spec.resources.requests.storage=="4Gi" and all((.pod.spec.containers + (.pod.spec.initContainers // []))[]; (.resources.limits // {} | has("ephemeral-storage") | not))' "$fixture"
 pod="$(jq -r '.names.pod' "$fixture")"
 home_claim="$(jq -r '.names.home' "$fixture")"
 scratch_claim="$(jq -r '.names.scratch' "$fixture")"
 # These are precisely the ResourceBuilder's real config/projections/services.
 jq '{apiVersion:"v1",kind:"List",items:([.namespace,.scratch,.service,.config]+.injections)}' "$fixture" | kube apply -f -
 identity="$(jq -r '.identity.metadata.name' "$fixture")"
 ssh-keygen -q -t ed25519 -N '' -f "$base/host-$case_id"
 kube create secret generic -n "$namespace" "$identity" \
  --from-file="ssh_host_ed25519_key=$base/host-$case_id" --from-file="ssh_host_ed25519_key.pub=$base/host-$case_id.pub"
 # Let real CSI publish fresh volumes, but reject the first sandbox using a
 # wrong thin-pool UUID in the correct VG. This observes the default knob
 # without resetting it between cases or bypassing the candidate wrapper.
 runtime_config=/etc/mwc-runtime-wrapper.json
 if [ "$runtime" = mwc-runsc ]; then runtime_config=/etc/mwc-runsc-wrapper.json; fi
 wrong_pool="LVM-${vg_uuid}00000000000000000000000000000000-tpool"
 test "$wrong_pool" != "$pool_uuid"
 writeback_state > "$case_out/writeback-before-pod.json"
 saved_config="$(mktemp "$runtime_config.positive.XXXXXX")"
 cp -p "$runtime_config" "$saved_config"
 restore_config="$saved_config"
 wrong_config="$(mktemp "$runtime_config.wrong-pool.XXXXXX")"
 jq --arg pool "$wrong_pool" '.scratch_pool_dm_uuid=$pool' "$restore_config" > "$wrong_config"
 mv "$wrong_config" "$runtime_config"
 errors="$base/state/$runtime/errors.log"
 error_offset=0
 if [ -f "$errors" ]; then error_offset="$(wc -l < "$errors")"; fi
 jq '.statefulset' "$fixture" | kube apply -f -
 for n in $(seq 1 150); do
  if kube get pod -n "$namespace" "$pod" -o json > "$case_out/writeback-rejected-pod.json" 2>/dev/null &&
     [ -s "$errors" ] &&
     tail -n "+$((error_offset+1))" "$errors" | grep -F 'expected exactly one trusted scratch publish, got 0' > "$case_out/writeback-rejected-errors.txt"; then break; fi
  sleep 2
 done
 test -s "$case_out/writeback-rejected-errors.txt"
 uid="$(jq -er '.metadata.uid' "$case_out/writeback-rejected-pod.json")"
 jq -e 'all(.status.conditions[]?; .type!="Ready" or .status!="True")' "$case_out/writeback-rejected-pod.json"
 scratch_uid="$(kube get pvc -n "$namespace" "$scratch_claim" -o jsonpath='{.metadata.uid}')"
 home_uid="$(kube get pvc -n "$namespace" "$home_claim" -o jsonpath='{.metadata.uid}')"
 scratch_pv="$(kube get pvc -n "$namespace" "$scratch_claim" -o jsonpath='{.spec.volumeName}')"
 home_pv="$(kube get pvc -n "$namespace" "$home_claim" -o jsonpath='{.spec.volumeName}')"
 kube get pv "$scratch_pv" "$home_pv" -o json > "$case_out/actual-pvs.json"
 jq -e 'all(.items[];.spec.csi.driver=="topolvm.io" and .status.phase=="Bound")' "$case_out/actual-pvs.json"
 publish="/var/lib/kubelet/pods/$uid/volumes/kubernetes.io~csi/$scratch_pv/mount"
 home_publish="/var/lib/kubelet/pods/$uid/volumes/kubernetes.io~csi/$home_pv/mount"
 pod_logs="$pod_log_root/${namespace}_${pod}_${uid}"
 writeback_state "$publish" "$home_publish" > "$case_out/writeback-rejected.json"
 jq -es '.[0].scope==.[1].scope and .[1].scratch.knobs.strict_limit==0 and .[1].home.knobs.strict_limit==0' \
  "$case_out/writeback-before-pod.json" "$case_out/writeback-rejected.json"
 test ! -e "$publish/runtime-private"
 test ! -L "$publish/runtime-private"
 test ! -L "$pod_logs"
 if grep -F "$publish/runtime-private/" /proc/self/mountinfo; then exit 1; fi
 restore_wrapper_config
 kube wait -n "$namespace" --for=condition=Ready "pod/$pod" --timeout=10m
 kube get pod -n "$namespace" "$pod" -o json > "$case_out/pod-initial.json"
 jq -e --arg uid "$uid" '.metadata.uid==$uid' "$case_out/pod-initial.json"
 check_active_writeback first-ready "$publish" "$home_publish"
 test -f "$(dirname "$publish")/vol_data.json"
 cp "$(dirname "$publish")/vol_data.json" "$case_out/actual-csi-vol-data.json"
 findmnt -J -T "$publish" > "$case_out/actual-publish-mount.json"
 test "$(findmnt -n -o FSTYPE -T "$publish")" = xfs
 for id in $(jq -r '[.status.containerStatuses[],.status.initContainerStatuses[]?]|.[].containerID|sub("containerd://";"")' "$case_out/pod-initial.json"); do
  cri inspect "$id" > "$case_out/container-$id.json"
  cp "$base/containerd-state/io.containerd.runtime.v2.task/k8s.io/$id/config.json" "$case_out/oci-$id.json" 2>/dev/null || true
 done
 # Actual CRI imageID/repoDigest and OCI configs are preserved. The public
 # workflow builds the workspace input; all four cases use that same image.
 for image in $(jq -r '[.pod.spec.containers[],.pod.spec.initContainers[]?]|.[].image' "$fixture" | sort -u); do
  cri inspecti "$image" > "$case_out/image-$(printf '%s' "$image" | sha256sum | cut -c1-12).json"
 done
 kube exec -n "$namespace" "$pod" -c workspace -- sh -ec 'rustc --version; cargo --version; printf home-survives > /home/user/storage-ci-home-marker; stat -c "%d %n" /home/user /tmp /var/tmp /var/lib/mwc/build-scratch; df -B1 / /home/user /tmp' > "$case_out/product-tools-storage.txt"
 # Use the original platform-created authorized client key; only this disposable
 # fixture's private key crosses to the runner, and it is never an artifact.
 kube exec -n "$namespace" "$pod" -c workspace -- cat /run/mwc-ssh/ttyd_client_key > "$base/client-$case_id"
 chmod 0600 "$base/client-$case_id"
 address="$(jq -r '.status.podIP' "$case_out/pod-initial.json")"
 printf '[%s]:2222 %s\n' "$address" "$(cut -d ' ' -f 1,2 "$base/host-$case_id.pub")" > "$base/known-$case_id"
 ssh_product() {
  ssh -F /dev/null -p 2222 -o BatchMode=yes -o StrictHostKeyChecking=yes -o ConnectTimeout=5 \
   -o "UserKnownHostsFile=$base/known-$case_id" -i "$base/client-$case_id" "user@$address" "$@"
 }
 ssh_product 'test "$(cat /home/user/storage-ci-home-marker)" = home-survives; printf strict-ssh-ok' > "$case_out/strict-ssh-before.txt"
 ttyd_path="$(jq -r '.pod.spec.containers[]|select(.name=="ttyd")|.args as $a|$a[($a|index("--base-path"))+1]' "$fixture")"
 curl -fsSL "http://$address:7681${ttyd_path}/" > "$case_out/ttyd-http.html"
 if [ "$enabled" = true ]; then
  # Actual rootless daemon security context, worker and layer generation.
  ssh_product 'mkdir -p /home/user/storage-ci-build; printf "FROM scratch\nCOPY marker /marker\n" > /home/user/storage-ci-build/Dockerfile; printf layer-proof > /home/user/storage-ci-build/marker; buildctl build --frontend dockerfile.v0 --local context=/home/user/storage-ci-build --local dockerfile=/home/user/storage-ci-build --output type=oci,dest=/home/user/storage-ci-layer.tar' > "$case_out/actual-buildkit-build.txt" 2>&1
 fi
 # A normal sidecar stop/delete must not remove another container's upper.
 main_id="$(jq -r '.status.containerStatuses[]|select(.name=="workspace")|.containerID|sub("containerd://";"")' "$case_out/pod-initial.json")"
 tty_id="$(jq -r '.status.containerStatuses[]|select(.name=="ttyd")|.containerID|sub("containerd://";"")' "$case_out/pod-initial.json")"
 cri stop "$tty_id"
 cri rm "$tty_id"
 check_active_writeback peer-deleted "$publish" "$home_publish"
 for n in $(seq 1 90); do
  replacement_tty="$(kube get pod -n "$namespace" "$pod" -o json | jq -r '.status.containerStatuses[]|select(.name=="ttyd" and .ready)|.containerID|sub("containerd://";"")')"
  if [ -n "$replacement_tty" ] && [ "$replacement_tty" != "$tty_id" ]; then break; fi
  sleep 1
 done
 test -n "$replacement_tty"; test "$replacement_tty" != "$tty_id"
 check_active_writeback peer-recreated "$publish" "$home_publish"
 test "$(kube get pod -n "$namespace" "$pod" -o json | jq -r '.status.containerStatuses[]|select(.name=="workspace")|.containerID|sub("containerd://";"")')" = "$main_id"
 ssh_product 'test "$(cat /home/user/storage-ci-home-marker)" = home-survives; printf main-survives-sidecar-delete' > "$case_out/strict-ssh-sidecar-replacement.txt"
 active_ids="$(kube get pod -n "$namespace" "$pod" -o json | jq '[.status.containerStatuses[].containerID|sub("containerd://";"")]')"
 jq -es --arg uid "$uid" --arg publish "$publish" --argjson ids "$active_ids" '
  [.[]|select(.pod_uid==$uid and (.id as $id|$ids|index($id)!=null))] |
  if length==($ids|length) and length>=2 and all(.[];.phase!="deleted" and .scratch.path==$publish and (.upper|startswith($publish+"/runtime-private/")) and all(.managed_files[]?;.redirected|startswith($publish+"/runtime-private/"))) then . else error("incorrect actual container scratch budget") end' "$base/state/$runtime/"*.json > "$case_out/actual-wrapper-records.json"
 for upper in $(jq -r '.[].upper' "$case_out/actual-wrapper-records.json"); do
  test "$(stat -Lc %d "$upper")" = "$(stat -Lc %d "$publish")"
 done
 kube exec -i -n "$namespace" "$pod" -c workspace -- sh -ec 'cat > /usr/local/bin/storage-ci-probe; chmod 0755 /usr/local/bin/storage-ci-probe' < "$RUNNER_TEMP/product-storage-bin/mwc-budget-probe"
 test -L "$pod_logs"
 test "$(readlink "$pod_logs")" = "$publish/runtime-private/logs/$uid"
 test "$(stat -Lc %d "$pod_logs")" = "$(stat -Lc %d "$publish")"
 findmnt -J -T "$pod_logs" > "$case_out/logs-shared-mount.json"
 summary() { kube get --raw /api/v1/nodes/mwc-product-ci/proxy/stats/summary; }
 check_identity() {
  kube get pod -n "$namespace" "$pod" -o json > "$case_out/pod-current.json"
  jq -e --arg uid "$uid" --arg id "containerd://$main_id" '.metadata.uid==$uid and .status.phase=="Running" and all(.status.containerStatuses[];.ready) and any(.status.containerStatuses[];.name=="workspace" and .containerID==$id)' "$case_out/pod-current.json" >/dev/null
 }
 # Use the real application PID 1 stdout, not kubectl exec's output stream.
 kube_log_emit() { timeout 90s "$base/k3s-bin" kubectl --kubeconfig "$base/kubeconfig" exec -n "$namespace" "$pod" -c workspace -- /usr/local/bin/storage-ci-probe emit /proc/1/fd/1 "$1" "$2"; }
 summary > "$case_out/summary-before-logs.json"
 timeout 240s "$base/k3s-bin" kubectl --kubeconfig "$base/kubeconfig" logs -n "$namespace" "$pod" -c workspace --follow --since=1s \
  > >(awk 'match($0,/MWC_LOG_[A-Z_]+/) {tag=substr($0,RSTART,RLENGTH); if (!seen[tag]++) {print tag; fflush()}}' > "$case_out/log-follow-markers.txt") & follow_pid=$!
 sleep 1
 kube_log_emit 67108864 MWC_LOG_BURST > "$case_out/log-burst.json"
 for n in $(seq 1 90); do
  if [ "$(find "$pod_logs/workspace" -maxdepth 1 -type f | wc -l)" -ge 2 ]; then break; fi
  check_identity
  sleep 2
 done
 test "$(find "$pod_logs/workspace" -maxdepth 1 -type f | wc -l)" -ge 2
 kube_log_emit 65536 MWC_LOG_REOPEN > "$case_out/log-reopen.json"
 logs_accounted=false
 for n in $(seq 1 30); do
  if grep -q MWC_LOG_REOPEN "$case_out/log-follow-markers.txt"; then break; fi
  sleep 1
 done
 grep -q MWC_LOG_BURST "$case_out/log-follow-markers.txt"
 grep -q MWC_LOG_REOPEN "$case_out/log-follow-markers.txt"
 kill "$follow_pid" 2>/dev/null || true
 wait "$follow_pid" 2>/dev/null || true
 for n in $(seq 1 30); do
  summary > "$case_out/summary-after-logs.json"
  if jq -e --arg uid "$uid" --slurpfile before "$case_out/summary-before-logs.json" '
   .pods[] | select(.podRef.uid==$uid) | . as $pod |
   .containers[] | select(.name=="workspace") | .logs.usedBytes as $logs |
   ($before[0].pods[] | select(.podRef.uid==$uid) | .containers[] | select(.name=="workspace") | .logs.usedBytes // 0) as $old |
   $logs>$old and $logs>1048576 and $pod["ephemeral-storage"].usedBytes >= $logs' "$case_out/summary-after-logs.json" >/dev/null; then logs_accounted=true; break; fi
  sleep 2
 done
 test "$logs_accounted" = true
 find -H "$pod_logs" -type f -printf '%P %s\n' > "$case_out/log-rotation-files.txt"
 test "$(du --summarize --block-size=1 --dereference "$pod_logs" | cut -f1)" -le 100663296
 kube_log_emit 65536 MWC_LOG_PREVIOUS > "$case_out/log-previous-emission.json"
 cri stop "$main_id"
 # Kubelet owns recreation and previous-log bookkeeping; do not delete its log.
 for n in $(seq 1 90); do
  new_main="$(kube get pod -n "$namespace" "$pod" -o json | jq -r '.status.containerStatuses[]|select(.name=="workspace" and .ready)|.containerID|sub("containerd://";"")')"
  if [ -n "$new_main" ] && [ "$new_main" != "$main_id" ]; then break; fi
  sleep 2
 done
 test -n "$new_main"; test "$new_main" != "$main_id"
 main_id="$new_main"
 check_identity
 kube logs -n "$namespace" "$pod" -c workspace --previous --limit-bytes=100663296 | awk '/MWC_LOG_PREVIOUS/ && !seen++ {print substr($0,1,180)}' > "$case_out/log-previous-markers.txt"
 grep -q MWC_LOG_PREVIOUS "$case_out/log-previous-markers.txt"
 ssh_product 'test "$(cat /home/user/storage-ci-home-marker)" = home-survives; printf strict-ssh-logs-ok' > "$case_out/strict-ssh-logs.txt"
 kube exec -i -n "$namespace" "$pod" -c workspace -- sh -ec 'cat > /usr/local/bin/storage-ci-probe; chmod 0755 /usr/local/bin/storage-ci-probe' < "$RUNNER_TEMP/product-storage-bin/mwc-budget-probe"
 diagnostic_case="$case_out"
 storage_diagnostics before-write
 check_active_writeback before-writer "$publish" "$home_publish"
 jq -es '
  .[0].scratch as $bdi | .[1].scratch_mount.device==$bdi.device and .[1].scratch_dm_uuid==$bdi.dm_uuid and
  all(.[2][]; .scratch.mount.device==$bdi.device and .scratch.dm_uuid==$bdi.dm_uuid)
 ' "$case_out/writeback-before-writer.json" "$case_out/runtime-storage-identity.json" "$case_out/actual-wrapper-records.json"
 storage_write write /storage-ci-root-fill 2415919104 > "$case_out/root-over-two-gib.json"
 jq -e '.written_bytes==2415919104 and .fsync_result==0 and .fsync_errno==0' "$case_out/root-over-two-gib.json"
 kube exec -n "$namespace" "$pod" -c workspace -- stat -c '%s %b' /storage-ci-root-fill > "$case_out/root-over-two-gib-stat.txt"
 read -r logical blocks < "$case_out/root-over-two-gib-stat.txt"
 test "$logical" = 2415919104
 test "$((blocks*512))" -ge 2415919104
 storage_diagnostics root-written
 # Cross several kubelet summary/eviction cycles with the same running Pod.
 for n in $(seq 1 30); do
  check_identity
  ssh_product 'test "$(cat /home/user/storage-ci-home-marker)" = home-survives' >/dev/null
  sleep 2
 done
 volume_accounted=false
 for n in $(seq 1 60); do
  summary > "$case_out/summary-over-two-gib.json"
  if jq -e --arg uid "$uid" --arg claim "$scratch_claim" '.pods[] | select(.podRef.uid==$uid) | any(.volume[]?; .pvcRef.name==$claim and .usedBytes>=2415919104)' "$case_out/summary-over-two-gib.json" >/dev/null; then volume_accounted=true; break; fi
  check_identity
  sleep 2
 done
 test "$volume_accounted" = true
 storage_write fill /storage-ci-root-fill > "$case_out/root-enospc.json"
 jq -e '.write_errno==28 or .write_errno==122' "$case_out/root-enospc.json"
 storage_diagnostics root-full
 kube exec -n "$namespace" "$pod" -c workspace -- /usr/local/bin/storage-ci-probe pwrite /tmp/storage-ci-temp-fill > "$case_out/tmp-enospc.json"
 kube exec -n "$namespace" "$pod" -c workspace -- /usr/local/bin/storage-ci-probe pwrite /var/lib/mwc/build-scratch/storage-ci-cache-fill > "$case_out/cache-enospc.json"
 kube exec -n "$namespace" "$pod" -c workspace -- sha256sum /etc/hosts > "$case_out/hosts-before.sha256"
 kube exec -n "$namespace" "$pod" -c workspace -- /usr/local/bin/storage-ci-probe pwrite /etc/hosts > "$case_out/hosts-enospc.json"
 jq -e '(.pwrite_errno==28 or .pwrite_errno==122) and .original_length_restored==true' "$case_out/hosts-enospc.json"
 kube exec -n "$namespace" "$pod" -c workspace -- sha256sum /etc/hosts > "$case_out/hosts-after.sha256"
 cmp "$case_out/hosts-before.sha256" "$case_out/hosts-after.sha256"
 # CRI stdout is asynchronous: assert bounded same-device logs and Pod survival,
 # without claiming that a process stdout write reports logger ENOSPC.
 kube_log_emit 1048576 MWC_LOG_FULL > "$case_out/log-full-emission.json"
 test "$(stat -Lc %d "$pod_logs")" = "$(stat -Lc %d "$publish")"
 test "$(du --summarize --block-size=1 --dereference "$pod_logs" | cut -f1)" -le 100663296
 kube exec -n "$namespace" "$pod" -c workspace -- sh -ec 'printf home-after-full > /home/user/storage-ci-after-full; test "$(cat /home/user/storage-ci-home-marker)" = home-survives'
 ssh_product 'test "$(cat /home/user/storage-ci-after-full)" = home-after-full; printf strict-ssh-full-ok' > "$case_out/strict-ssh-full.txt"
 for n in $(seq 1 15); do check_identity; sleep 2; done
 cp "$case_out/pod-current.json" "$case_out/pod-full.json"
 kube get events -n "$namespace" --field-selector "involvedObject.uid=$uid" -o json > "$case_out/pod-events.json"
 jq -e 'all(.items[]; .reason!="Evicted" and .reason!="EvictionThresholdMet")' "$case_out/pod-events.json"
 # Free fixture data before normal replacement; do not change/replace either PVC.
 kube exec -n "$namespace" "$pod" -c workspace -- sh -ec 'rm -f /storage-ci-root-fill /tmp/storage-ci-temp-fill /var/lib/mwc/build-scratch/storage-ci-cache-fill'
 kube delete pod -n "$namespace" "$pod" --wait=true --timeout=120s
 for n in $(seq 1 180); do
  new_uid="$(kube get pod -n "$namespace" "$pod" -o jsonpath='{.metadata.uid}' 2>/dev/null || true)"
  if [ -n "$new_uid" ] && [ "$new_uid" != "$uid" ]; then break; fi
  sleep 1
 done
 test -n "$new_uid"; test "$new_uid" != "$uid"
 kube wait -n "$namespace" --for=condition=Ready "pod/$pod" --timeout=300s
 test "$(kube get pvc -n "$namespace" "$scratch_claim" -o jsonpath='{.metadata.uid}')" = "$scratch_uid"
 test "$(kube get pvc -n "$namespace" "$home_claim" -o jsonpath='{.metadata.uid}')" = "$home_uid"
 kube get pod -n "$namespace" "$pod" -o json > "$case_out/pod-replacement.json"
 check_active_writeback republished "/var/lib/kubelet/pods/$new_uid/volumes/kubernetes.io~csi/$scratch_pv/mount" \
  "/var/lib/kubelet/pods/$new_uid/volumes/kubernetes.io~csi/$home_pv/mount"
 address="$(jq -r '.status.podIP' "$case_out/pod-replacement.json")"
 printf '[%s]:2222 %s\n' "$address" "$(cut -d ' ' -f 1,2 "$base/host-$case_id.pub")" > "$base/known-$case_id"
 kube exec -n "$namespace" "$pod" -c workspace -- cat /run/mwc-ssh/ttyd_client_key > "$base/client-$case_id"
 ssh_product 'test "$(cat /home/user/storage-ci-home-marker)" = home-survives; printf strict-ssh-new-pod-ok' > "$case_out/strict-ssh-replacement.txt"
 # Real kubelet + CSI own unpublish; no fixture umount/vol_data manipulation.
 kube delete statefulset -n "$namespace" "${pod%-0}" --wait=true --timeout=120s
 kube wait -n "$namespace" --for=delete "pod/$pod" --timeout=180s
 for n in $(seq 1 120); do
  if ! findmnt -rn -o TARGET | grep -F "/var/lib/kubelet/pods/$new_uid/" >/dev/null; then break; fi
  sleep 1
 done
 if findmnt -rn -o TARGET | grep -F -e "/var/lib/kubelet/pods/$new_uid/" -e "/var/lib/kubelet/pods/$uid/"; then exit 1; fi
 handles="$(jq -r '.items[].spec.csi.volumeHandle' "$case_out/actual-pvs.json")"
 kube delete pvc -n "$namespace" "$home_claim" "$scratch_claim" --wait=true --timeout=120s
 kube wait --for=delete "pv/$home_pv" "pv/$scratch_pv" --timeout=180s
 for n in $(seq 1 120); do
  remaining=0
  for handle in $handles; do if lvs --noheadings -o lv_name | tr -d ' ' | grep -Fx "$handle" >/dev/null; then remaining=1; fi; done
  if [ "$remaining" = 0 ]; then break; fi
  sleep 1
 done
 test "$remaining" = 0
 # A major:minor may be allocated again; it must not still identify this LV.
 scratch_device="$(jq -r .scratch.device "$case_out/writeback-republished.json")"
 scratch_dm_uuid="$(jq -r .scratch.dm_uuid "$case_out/writeback-republished.json")"
 dm_uuid_path="/sys/dev/block/$scratch_device/dm/uuid"
 remaining_dm_uuid=''
 if [ -e "$dm_uuid_path" ]; then remaining_dm_uuid="$(cat "$dm_uuid_path")"; fi
 test "$remaining_dm_uuid" != "$scratch_dm_uuid"
 writeback_state | jq --arg device "$scratch_device" --arg old "$scratch_dm_uuid" --arg observed "$remaining_dm_uuid" \
  '. + {old_scratch_device:$device,old_scratch_dm_uuid:$old,observed_dm_uuid:(if $observed=="" then null else $observed end)}' \
  > "$case_out/writeback-after-lv-delete.json"
 jq -es '.[0].scope==.[1].scope' "$case_out/writeback-before-pod.json" "$case_out/writeback-after-lv-delete.json"
 kube delete secret -n "$namespace" "$identity" --wait=true --timeout=120s
 jq -n --arg runtime "$runtime" --argjson buildkit "$enabled" --arg uid "$uid" --arg new_uid "$new_uid" --arg scratch_uid "$scratch_uid" --arg home_uid "$home_uid" --arg scratch_dm_uuid "$scratch_dm_uuid" \
  '{accepted:true,runtime:$runtime,buildkit:$buildkit,writeback_wrapper_owned:true,wrong_pool_rejected_without_tuning:true,writeback_peer_and_republish_readback:true,root_written_bytes:2415919104,root_fsync:true,scratch_enospc:true,logs_rotation:true,logs_follow_reopen:true,logs_previous:true,kubelet_logs_accounted:true,kubelet_scratch_accounted:true,logs_share_scratch:true,ssh_survived:true,pod_identity_preserved_under_pressure:true,old_pod_uid:$uid,new_pod_uid:$new_uid,scratch_pvc_uid:$scratch_uid,scratch_dm_uuid:$scratch_dm_uuid,home_pvc_uid:$home_uid,pvc_reused:true,node_unpublish_observed:true,pv_lv_deleted:true,crash_reboot_proven:false,all_templates_proven:false,eviction_settings_changed:false}' > "$case_out/acceptance.json"
 diagnostic_case=''
done
done
test "$(kube get pod mwc-default-runtime -o jsonpath='{.metadata.uid}:{.status.containerStatuses[0].containerID}:{.status.containerStatuses[0].restartCount}')" = "$default_uid:containerd://$default_id:0"
test "$(kube exec mwc-default-runtime -- cat /tmp/mwc-sentinel)" = ordinary-runtime
test "$(cat "$pod_log_root/existing-marker")" = original-log-directory
test "$(stat -c %d:%i "$pod_log_root")" = "$log_inode"
kube delete pod mwc-default-runtime --wait=true --timeout=120s
for n in $(seq 1 30); do
 if ! findmnt -n --mountpoint "$default_rootfs" >/dev/null; then break; fi
 sleep 1
done
if findmnt -n --mountpoint "$default_rootfs" >/dev/null; then exit 1; fi
snapshot
jq -es --arg renderer "$PRODUCT_RENDERER_SHA" --arg wrapper "$WRAPPER_SHA" --arg product "$(jq -r '.pod.spec.containers[]|select(.name=="workspace")|.image' "$fixtures/mwc-native-buildkit-false.json")" '
 sort_by(.runtime,.buildkit) as $cases |
 if ($cases|length)==4 and ([$cases[]|[.runtime,.buildkit]]==[["mwc-native",false],["mwc-native",true],["mwc-runsc",false],["mwc-runsc",true]]) and
 ([$cases[].scratch_dm_uuid] | unique | length)==4 and
 all($cases[]; .accepted and .writeback_wrapper_owned and .wrong_pool_rejected_without_tuning and .writeback_peer_and_republish_readback and .root_written_bytes>2147483648 and .root_fsync and .scratch_enospc and .logs_rotation and .logs_follow_reopen and .logs_previous and .kubelet_logs_accounted and .kubelet_scratch_accounted and .logs_share_scratch and .ssh_survived and .pod_identity_preserved_under_pressure and .pvc_reused and .node_unpublish_observed and .pv_lv_deleted)
 then {accepted:true,renderer_sha:$renderer,wrapper_sha:$wrapper,product:$product,cases:$cases,real_csi:true,production_deployed:false,runtime_matrix_complete:true,writeback_policy:"wrapper-strict-limit",mount_layout:"shared-parent-private-bundle",canonical_log_leaf_pinned:true,default_runtime_survived:true,default_runtime_cleanup:true}
 else error("incomplete real CSI quota runtime matrix") end' "$out/"mwc-*/acceptance.json > "$out/acceptance.json"
