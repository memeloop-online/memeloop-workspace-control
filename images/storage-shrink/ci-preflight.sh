#!/bin/bash
set -euo pipefail
export LC_ALL=C

for tool in tar python3 du df cut tail tr ln truncate sha256sum; do
  command -v "$tool" >/dev/null
done
tar --version | grep -i 'GNU tar'
tar_help="$(tar --help)"
for option in --sparse --xattrs --acls --numeric-owner --one-file-system; do
  grep -q -- "${option}" <<<"${tar_help}"
done

for script in /runtime-preflight.sh /runtime-measure.sh /runtime-copy.sh /runtime-verify.sh; do
  test -f "${script}"
done

mkdir -p /source/dir /destination
echo payload > /source/dir/data
echo plain > /source/dir/plain
ln /source/dir/data /source/hardlink
truncate -s 64M /source/sparse
ln -s dir/data /source/symlink

python3 -I - <<'PYTHON'
import os
import struct

def entry(tag, perm, ident):
    return struct.pack("<HHI", tag, perm, ident)

data = "/source/dir/data"
os.chown(data, 1234, 5678)
os.setxattr(data, "user.mwc-preflight", b"storage-shrink", follow_symlinks=False)

access_acl = struct.pack("<I", 2) + b"".join([
    entry(0x01, 0o6, 0xFFFFFFFF),
    entry(0x02, 0o6, 0),
    entry(0x04, 0o4, 0xFFFFFFFF),
    entry(0x10, 0o6, 0xFFFFFFFF),
    entry(0x20, 0o0, 0xFFFFFFFF),
])
os.setxattr(data, "system.posix_acl_access", access_acl, follow_symlinks=False)
assert "system.posix_acl_access" in os.listxattr(data, follow_symlinks=False)

default_acl = struct.pack("<I", 2) + b"".join([
    entry(0x01, 0o7, 0xFFFFFFFF),
    entry(0x02, 0o4, 1001),
    entry(0x04, 0o5, 0xFFFFFFFF),
    entry(0x10, 0o5, 0xFFFFFFFF),
    entry(0x20, 0o0, 0xFFFFFFFF),
])
os.setxattr("/source/dir", "system.posix_acl_default", default_acl, follow_symlinks=False)
assert "system.posix_acl_default" in os.listxattr("/source/dir", follow_symlinks=False)

os.utime(data, ns=(1700000000123456789, 1700000000123456789), follow_symlinks=False)
PYTHON

: > /tmp/copy-source-errors
: > /tmp/copy-destination-errors
: > /tmp/verification-result
: > /tmp/verification-errors

bash /runtime-measure.sh
target_bytes="$(df -B1 --output=size /source | tail -1 | tr -d '[:space:]')"
TARGET_BYTES="${target_bytes}" bash /runtime-copy.sh
TARGET_BYTES="${target_bytes}" bash /runtime-verify.sh

python3 -I - <<'PYTHON'
import hashlib
import os

def digest(path):
    checksum = hashlib.sha256()
    with open(path, "rb") as content:
        for block in iter(lambda: content.read(1024 * 1024), b""):
            checksum.update(block)
    return checksum.digest()

source_data = "/source/dir/data"
copied_data = "/destination/dir/data"
copied_hardlink = "/destination/hardlink"
copied_sparse = "/destination/sparse"
copied_symlink = "/destination/symlink"

assert digest(source_data) == digest(copied_data), "file content mismatch"
assert os.getxattr(copied_data, "user.mwc-preflight", follow_symlinks=False) == b"storage-shrink", \
    "user xattr lost"
assert os.getxattr(copied_data, "system.posix_acl_access", follow_symlinks=False) == \
    os.getxattr(source_data, "system.posix_acl_access", follow_symlinks=False), \
    "POSIX ACL lost"
assert os.getxattr("/destination/dir", "system.posix_acl_default", follow_symlinks=False) == \
    os.getxattr("/source/dir", "system.posix_acl_default", follow_symlinks=False), \
    "directory default ACL lost"

source_stat = os.lstat(source_data)
copied_stat = os.lstat(copied_data)
assert (copied_stat.st_uid, copied_stat.st_gid) == (1234, 5678), \
    "numeric ownership lost"
assert copied_stat.st_mtime_ns == source_stat.st_mtime_ns, \
    "nanosecond mtime lost"

hardlink_stat = os.stat(copied_hardlink)
assert hardlink_stat.st_nlink == 2, "hardlink count lost"
assert (hardlink_stat.st_dev, hardlink_stat.st_ino) == \
    (copied_stat.st_dev, copied_stat.st_ino), "hardlink group split"

assert os.readlink(copied_symlink) == os.readlink("/source/symlink"), \
    "symlink target mismatch"
assert os.lstat(copied_symlink).st_mtime_ns == os.lstat("/source/symlink").st_mtime_ns, \
    "symlink mtime lost"

assert digest("/source/sparse") == digest(copied_sparse), "sparse content mismatch"
sparse_stat = os.stat(copied_sparse)
assert sparse_stat.st_size == 64 * 1024 * 1024, "sparse size mismatch"
assert sparse_stat.st_blocks * 512 < sparse_stat.st_size, \
    "sparse file was materialized"
PYTHON

echo "storage shrink helper toolchain and runtime copy/verify preflight passed"
