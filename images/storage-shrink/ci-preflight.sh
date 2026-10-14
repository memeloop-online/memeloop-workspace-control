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

TARGET_BYTES="${target_bytes}" python3 -I - <<'PYTHON'
import os
import stat
import subprocess

def verify(policy, succeeds, label):
    environment = os.environ.copy()
    environment.pop("MOUNT_ROOT_MTIME_POLICY", None)
    if policy is not None:
        environment["MOUNT_ROOT_MTIME_POLICY"] = policy
    result = subprocess.run(
        ["bash", "/runtime-verify.sh"],
        env=environment,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    )
    if (result.returncode == 0) != succeeds:
        raise RuntimeError(f"{label}: unexpected verification exit code {result.returncode}")

def reject_change(label, change, restore):
    try:
        change()
        verify("longhorn-probe", False, label)
    finally:
        restore()
    verify("longhorn-probe", True, f"{label} restored")

root = "/destination"
root_metadata = os.lstat(root)
try:
    os.utime(root, ns=(root_metadata.st_atime_ns, root_metadata.st_mtime_ns + 1000000000))
    verify(None, False, "default policy rejects mount-root mtime mismatch")
    verify("strict", False, "strict policy rejects mount-root mtime mismatch")
    verify("unknown", False, "unknown policy rejects mount-root mtime mismatch")
    verify("longhorn-probe", True, "longhorn-probe allows mount-root mtime mismatch")
finally:
    os.utime(root, ns=(root_metadata.st_atime_ns, root_metadata.st_mtime_ns))

verify(None, True, "mount-root mtime restored")
verify("strict", True, "strict policy accepts restored mount-root mtime")
verify("unknown", False, "unknown policy rejects matching fixtures")

for path, label in (
    ("/destination/dir/plain", "child file mtime mismatch"),
    ("/destination/dir", "child directory mtime mismatch"),
    ("/destination/symlink", "symlink mtime mismatch"),
):
    metadata = os.lstat(path)
    reject_change(
        label,
        lambda: os.utime(path, ns=(metadata.st_atime_ns, metadata.st_mtime_ns + 1000000000),
                         follow_symlinks=False),
        lambda: os.utime(path, ns=(metadata.st_atime_ns, metadata.st_mtime_ns),
                         follow_symlinks=False),
    )

reject_change(
    "mount-root mode mismatch",
    lambda: os.chmod(root, stat.S_IMODE(root_metadata.st_mode) ^ stat.S_IXOTH),
    lambda: os.chmod(root, stat.S_IMODE(root_metadata.st_mode)),
)
reject_change(
    "mount-root uid mismatch",
    lambda: os.chown(root, root_metadata.st_uid + 1, -1),
    lambda: os.chown(root, root_metadata.st_uid, -1),
)
reject_change(
    "mount-root gid mismatch",
    lambda: os.chown(root, -1, root_metadata.st_gid + 1),
    lambda: os.chown(root, -1, root_metadata.st_gid),
)

attribute = "user.mwc-preflight"
root_attribute = (
    os.getxattr(root, attribute, follow_symlinks=False)
    if attribute in os.listxattr(root, follow_symlinks=False) else None
)
reject_change(
    "mount-root user xattr mismatch",
    lambda: os.setxattr(root, attribute, (root_attribute or b"") + b"changed",
                       follow_symlinks=False),
    lambda: os.removexattr(root, attribute, follow_symlinks=False)
    if root_attribute is None else os.setxattr(root, attribute, root_attribute,
                                              follow_symlinks=False),
)

for path, attribute, label in (
    ("/destination/dir/data", "system.posix_acl_access", "child file ACL mismatch"),
    ("/destination/dir", "system.posix_acl_default", "child directory default ACL mismatch"),
):
    original_acl = os.getxattr(path, attribute, follow_symlinks=False)
    reject_change(
        label,
        lambda: os.removexattr(path, attribute, follow_symlinks=False),
        lambda: os.setxattr(path, attribute, original_acl, follow_symlinks=False),
    )

path = "/destination/dir/plain"
metadata = os.lstat(path)
with open(path, "rb") as content:
    original_content = content.read()
try:
    with open(path, "r+b") as content:
        content.write(bytes([original_content[0] ^ 1]))
    os.utime(path, ns=(metadata.st_atime_ns, metadata.st_mtime_ns), follow_symlinks=False)
    verify("longhorn-probe", False, "same-size child content mismatch with preserved mtime")
finally:
    with open(path, "r+b") as content:
        content.write(original_content)
    os.utime(path, ns=(metadata.st_atime_ns, metadata.st_mtime_ns), follow_symlinks=False)

verify(None, True, "restored fixtures pass default verification")
PYTHON

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
