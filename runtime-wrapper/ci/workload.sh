#!/bin/bash
set -euo pipefail
case "${1:-serve}" in
 serve)
  echo "workspace-stdout-start"
  echo "workspace-stderr-start" >&2
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
