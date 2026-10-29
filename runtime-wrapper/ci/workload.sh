#!/bin/bash
set -euo pipefail
case "${1:-serve}" in
 serve)
  mkdir -p /run/sshd
  # CI has independent Home; host keys/auth were provisioned there before create.
  exec /usr/sbin/sshd -D -e -p 2222 -o HostKey=/home/user/host_ed25519 -o AuthorizedKeysFile=/home/user/authorized_keys -o PasswordAuthentication=no -o UsePAM=no -o PidFile=/home/user/sshd.pid
  ;;
 idle) exec sleep infinity ;;
 init)
  python3 - <<'PY'
with open('/init-allocation','wb') as f:
 f.write(b'i' * (8 * 1024 * 1024)); f.flush()
 import os; os.fsync(f.fileno())
PY
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
