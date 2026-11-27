#!/bin/bash
set -euo pipefail
case "${1:-serve}" in
 serve|runsc-serve)
  echo "workspace-stdout-start"
  echo "workspace-stderr-start" >&2
  if [ "$1" = runsc-serve ]; then
   # Write the inherited CRI stdout directly. Reopening /proc/1/fd/1 is not
   # supported by gVisor; this workload needs no additional capabilities.
   python3 - <<'PYLOG' &
import os,pathlib,time
trigger=pathlib.Path('/home/user/log-burst-trigger')
while not trigger.exists():time.sleep(0.1)
for _ in range(80000):os.write(1,b'L'*8191+b'\n')
pathlib.Path('/home/user/log-burst-done').write_text('inherited stdout complete\n')
PYLOG
  fi
  mkdir -p /run/sshd
  # CI has independent Home; host keys/auth were provisioned there before create.
  exec /usr/sbin/sshd -D -e -p 2222 -o HostKey=/home/user/host_ed25519 -o AuthorizedKeysFile=/home/user/authorized_keys -o PasswordAuthentication=no -o UsePAM=no -o PidFile=/home/user/sshd.pid
  ;;
 idle) echo "sidecar-stdout-start"; echo "sidecar-stderr-start" >&2; exec sleep infinity ;;
 init)
  echo "init-stdout-start"
  echo "init-stderr-start" >&2
  python3 - <<'PY'
with open('/init-allocation','wb') as f:
 f.write(b'i' * (8 * 1024 * 1024)); f.flush()
 import os; os.fsync(f.fileno())
PY
  ;;
 readonly-check)
  python3 - <<'PYRO'
import errno,json,os
result={}
for path in ['/seed-copy-up','/readonly-root-create']:
 try:
  with open(path,'ab') as f:f.write(b'root must remain readonly')
 except OSError as e:
  assert e.errno==errno.EROFS,(path,e)
  result[path]={'errno':e.errno}
 else:raise AssertionError('readonly rootfs accepted a write: '+path)
for path in ['/scratch/readonly-marker','/home/user/readonly-marker']:
 with open(path,'w') as f:f.write('readonly-sidecar-volume-writable\n');f.flush();os.fsync(f.fileno())
 result[path]={'writable':True}
status=dict(line.split(':',1) for line in open('/proc/self/status') if ':' in line)
assert not int(status['CapEff'].strip(),16)&(1<<21),'SYS_ADMIN was granted'
assert status['NoNewPrivs'].strip()=='1',status['NoNewPrivs']
result['SYS_ADMIN_present']=False
result['NoNewPrivs']=True
print(json.dumps(result))
PYRO
  ;;
 runsc-fill)
  python3 - <<'PYRF'
import errno,json,mmap,os,signal,subprocess,sys
result={}
paths=['/rootfs-fill','/scratch/data-fill','/root-small','/scratch/small','/root-pwrite','/scratch/pwrite','/etc/hosts','/etc/hostname','/etc/resolv.conf']
fds={p:os.open(p,os.O_WRONLY|os.O_CREAT|(os.O_APPEND if p.startswith('/etc/') else 0),0o644) for p in paths}
with open('/mmap-probe','wb') as f:f.truncate(4096)
def fill(path,block):
 count=0
 try:
  while True:count+=os.write(fds[path],block)
 except OSError as e:
  assert e.errno in (errno.ENOSPC,errno.EDQUOT),(path,e)
  result[path]={'bytes':count,'errno':e.errno,'operation':'write'}
for path in ['/rootfs-fill','/scratch/data-fill']:fill(path,b'x'*1048576)
for path in ['/root-small','/scratch/small','/etc/hosts','/etc/hostname','/etc/resolv.conf']:fill(path,b'x'*4096)
for path in ['/root-pwrite','/scratch/pwrite']:
 try:os.pwrite(fds[path],b'p'*4096,0)
 except OSError as e:
  assert e.errno in (errno.ENOSPC,errno.EDQUOT),(path,e)
  result[path]={'errno':e.errno,'operation':'pwrite'}
 else:raise AssertionError('pwrite bypassed full shared filesystem: '+path)
for path,fd in fds.items():
 try:os.fsync(fd);outcome={'return':0}
 except OSError as e:
  assert e.errno in (errno.ENOSPC,errno.EDQUOT),(path,e)
  outcome={'errno':e.errno}
 result[path]['fsync']=outcome
 os.close(fd)
# Isolate a possible SIGBUS to one application process. A successful write with
# no filesystem error on a full hole would be a failing acceptance, not proof.
probe=subprocess.run([sys.executable,'-c','import mmap;f=open("/mmap-probe","r+b");m=mmap.mmap(f.fileno(),4096);m[0]=1;m.flush()'],capture_output=True)
result['mmap']={'returncode':probe.returncode,'stderr':probe.stderr.decode(errors='replace')}
assert probe.returncode!=0,'mmap wrote full sparse rootfs without error'
assert probe.returncode==-signal.SIGBUS or any(s in result['mmap']['stderr'] for s in ['No space left','Disk quota exceeded']),result['mmap']
with open('/home/user/runsc-full-result.json','w') as f:json.dump(result,f);f.flush();os.fsync(f.fileno())
print(json.dumps(result))
PYRF
  ;;
 fill)
  python3 - <<'PY'
import errno,json,os
result={}
for path in ['/rootfs-fill','/scratch/data-fill','/etc/hosts','/etc/hostname','/etc/resolv.conf']:
 count=0
 try:
  with open(path,'ab' if path.startswith('/etc/') else 'wb',buffering=0) as f:
   while True:
    count+=f.write(b'x'*(1024*1024))
    os.fsync(f.fileno())
 except OSError as e:
  assert e.errno in (errno.ENOSPC,errno.EDQUOT),e
  result[path]={'bytes':count,'errno':e.errno,'message':str(e)}
with open('/home/user/full-result.json','w') as f:json.dump(result,f)
with open('/home/user/home-still-writable','w') as f:f.write('home survives scratch-full\n');f.flush();os.fsync(f.fileno())
print(json.dumps(result))
PY
 ;;
 *) exec "$@" ;;
esac
