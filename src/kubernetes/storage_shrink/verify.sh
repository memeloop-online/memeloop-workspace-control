set -euo pipefail
export LC_ALL=C
for tool in tar python3 du df; do command -v "$tool" >/dev/null; done
used_bytes=$(du -sx --block-size=1 /source | cut -f1)
copy_bytes=$used_bytes
filesystem_used=$(df -B1 --output=used /source | tail -1 | tr -d '[:space:]')
if [ "$filesystem_used" -gt "$used_bytes" ]; then used_bytes=$filesystem_used; fi
used_inodes=$(python3 -I - <<'PYTHON'
import os
import stat
count = 1
seen_links = set()
def fail_walk(error):
    raise error
for directory, directories, files in os.walk("/source", followlinks=False, onerror=fail_walk):
    for name in directories + files:
        metadata = os.lstat(os.path.join(directory, name))
        inode = (metadata.st_dev, metadata.st_ino)
        if metadata.st_nlink > 1 and not stat.S_ISDIR(metadata.st_mode):
            if inode in seen_links:
                continue
            seen_links.add(inode)
        count += 1
print(count)
PYTHON
)
test "$TARGET_BYTES" -gt "$used_bytes"
python3 -I - <<'PYTHON' >/tmp/verification-result 2>/tmp/verification-errors
import hashlib
import os
import stat

source_links = {}
destination_links = {}
root_mtime_policy = os.environ.get("MOUNT_ROOT_MTIME_POLICY", "strict")
if root_mtime_policy not in ("strict", "longhorn-probe"):
    raise RuntimeError("unsupported mount-root mtime policy")

def digest(path):
    checksum = hashlib.sha256()
    with open(path, "rb") as content:
        for block in iter(lambda: content.read(1024 * 1024), b""):
            checksum.update(block)
    return checksum.digest()

def attributes(path):
    names = os.listxattr(path, follow_symlinks=False)
    if any(not name.startswith("user.") and name not in ("system.posix_acl_access", "system.posix_acl_default") for name in names):
        raise RuntimeError("unsupported visible xattr namespace")
    return {name: os.getxattr(path, name, follow_symlinks=False) for name in names}

def verify(source, destination):
    source_stat = os.lstat(source)
    destination_stat = os.lstat(destination)
    assert (source_stat.st_mode, source_stat.st_uid, source_stat.st_gid) == (
            destination_stat.st_mode, destination_stat.st_uid,
            destination_stat.st_gid), "metadata mismatch"
    if source != "/source" or root_mtime_policy != "longhorn-probe":
        assert source_stat.st_mtime_ns == destination_stat.st_mtime_ns, "mtime mismatch"
    assert attributes(source) == attributes(destination), "xattrs/ACL mismatch"
    if not stat.S_ISDIR(source_stat.st_mode):
        assert source_stat.st_nlink == destination_stat.st_nlink, "hardlink count mismatch"
        if source_stat.st_nlink > 1:
            source_inode = (source_stat.st_dev, source_stat.st_ino)
            destination_inode = (destination_stat.st_dev, destination_stat.st_ino)
            assert source_links.setdefault(source_inode, destination_inode) == destination_inode, "split hardlink group"
            assert destination_links.setdefault(destination_inode, source_inode) == source_inode, "merged hardlink group"
    if stat.S_ISREG(source_stat.st_mode):
        assert source_stat.st_size == destination_stat.st_size, "file size mismatch"
        assert digest(source) == digest(destination), "SHA-256 mismatch"
    elif stat.S_ISLNK(source_stat.st_mode):
        assert os.readlink(source) == os.readlink(destination), "symlink mismatch"
    elif stat.S_ISDIR(source_stat.st_mode):
        source_entries = set(os.listdir(source))
        assert source_entries == set(os.listdir(destination)), "directory entries mismatch"
        for name in sorted(source_entries):
            verify(os.path.join(source, name), os.path.join(destination, name))
    else:
        assert source_stat.st_rdev == destination_stat.st_rdev, "special file mismatch"

verify("/source", "/destination")
PYTHON
printf '{"used_bytes":%s,"used_inodes":%s}' "$used_bytes" "$used_inodes" > /dev/termination-log
