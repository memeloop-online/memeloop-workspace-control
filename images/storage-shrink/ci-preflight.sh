#!/bin/bash
set -euo pipefail
export LC_ALL=C

for tool in tar python3 du df cut tail tr install ln mktemp truncate sha256sum; do
  command -v "$tool" >/dev/null
done
tar --version | grep -i 'GNU tar'
tar_help="$(tar --help)"
for option in --sparse --xattrs --acls --numeric-owner --one-file-system; do
  grep -q -- "${option}" <<<"${tar_help}"
done

work="$(mktemp -d)"
trap 'rm -rf "${work}"' EXIT
mkdir -p "${work}/source/dir" "${work}/destination"
echo payload > "${work}/source/dir/data"
ln "${work}/source/dir/data" "${work}/source/hardlink"
truncate -s 64M "${work}/source/sparse"

python3 -I - "${work}/source/dir/data" <<'PYTHON'
import os
import struct
import sys

path = sys.argv[1]
os.setxattr(path, "user.mwc-preflight", b"storage-shrink", follow_symlinks=False)

def entry(tag, perm, ident):
    return struct.pack("<HHI", tag, perm, ident)

acl = struct.pack("<I", 2) + b"".join([
    entry(0x01, 0o6, 0xFFFFFFFF),
    entry(0x02, 0o6, 0),
    entry(0x04, 0o4, 0xFFFFFFFF),
    entry(0x10, 0o6, 0xFFFFFFFF),
    entry(0x20, 0o0, 0xFFFFFFFF),
])
os.setxattr(path, "system.posix_acl_access", acl, follow_symlinks=False)
assert "system.posix_acl_access" in os.listxattr(path, follow_symlinks=False)
PYTHON

tar --create --file=- --format=pax --directory="${work}/source" \
    --one-file-system --sparse --xattrs --xattrs-include='*' --acls \
    --numeric-owner . \
  | tar --extract --file=- --directory="${work}/destination" --sparse \
    --xattrs --xattrs-include='*' --acls --numeric-owner --same-owner \
    --same-permissions --delay-directory-restore

python3 -I - "${work}/source" "${work}/destination" <<'PYTHON'
import hashlib
import os
import sys

source, destination = sys.argv[1], sys.argv[2]

def digest(path):
    checksum = hashlib.sha256()
    with open(path, "rb") as content:
        for block in iter(lambda: content.read(1024 * 1024), b""):
            checksum.update(block)
    return checksum.digest()

source_data = os.path.join(source, "dir", "data")
copied_data = os.path.join(destination, "dir", "data")
copied_hardlink = os.path.join(destination, "hardlink")
copied_sparse = os.path.join(destination, "sparse")

assert digest(source_data) == digest(copied_data), "file content mismatch"
assert os.getxattr(copied_data, "user.mwc-preflight", follow_symlinks=False) == b"storage-shrink", \
    "user xattr lost"
assert os.getxattr(copied_data, "system.posix_acl_access", follow_symlinks=False) == \
    os.getxattr(source_data, "system.posix_acl_access", follow_symlinks=False), \
    "POSIX ACL lost"

data_stat = os.stat(copied_data)
hardlink_stat = os.stat(copied_hardlink)
assert hardlink_stat.st_nlink == 2, "hardlink count lost"
assert (hardlink_stat.st_dev, hardlink_stat.st_ino) == \
    (data_stat.st_dev, data_stat.st_ino), "hardlink group split"

source_sparse = os.path.join(source, "sparse")
assert digest(source_sparse) == digest(copied_sparse), "sparse content mismatch"
sparse_stat = os.stat(copied_sparse)
assert sparse_stat.st_size == 64 * 1024 * 1024, "sparse size mismatch"
assert sparse_stat.st_blocks * 512 < sparse_stat.st_size, \
    "sparse file was materialized"
PYTHON

echo "storage shrink helper toolchain preflight passed"
