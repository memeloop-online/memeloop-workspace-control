set -euo pipefail
export LC_ALL=C
for tool in tar python3 du df cut tail tr; do command -v "$tool" >/dev/null; done
tar --help > /tmp/tar-help
python3 -I - <<'PYTHON'
import hashlib
import os
import stat
with open("/tmp/tar-help") as content:
    help_text = content.read()
assert all(option in help_text for option in ("--sparse", "--xattrs", "--acls", "--numeric-owner", "--one-file-system"))
assert all(hasattr(os, name) for name in ("listxattr", "getxattr", "lstat", "walk"))
assert len(hashlib.sha256(b"preflight").digest()) == 32
PYTHON
printf '{"used_bytes":0,"used_inodes":0}' > /dev/termination-log
