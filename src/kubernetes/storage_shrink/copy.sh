set -euo pipefail
export LC_ALL=C
failure_step=tools
failure_details=/tmp/copy-failure
: > "$failure_details"
trap 'status=$?; if [ "$status" -ne 0 ]; then if [ -s "$failure_details" ]; then cat "$failure_details" > /dev/termination-log; else printf "{\"stage\":\"copy\",\"step\":\"%s\",\"exit_code\":%s}" "$failure_step" "$status" > /dev/termination-log; fi; fi' EXIT
for tool in tar python3 du df; do command -v "$tool" >/dev/null; done
failure_step=capacity
used_bytes=$(du -sx --block-size=1 /source | cut -f1)
copy_bytes=$used_bytes
filesystem_used=$(df -B1 --output=used /source | tail -1 | tr -d '[:space:]')
if [ "$filesystem_used" -gt "$used_bytes" ]; then used_bytes=$filesystem_used; fi
failure_step=inventory
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
failure_step=capacity
test "$TARGET_BYTES" -gt "$used_bytes"
available_bytes=$(df -B1 --output=avail /destination | tail -1 | tr -d '[:space:]')
available_inodes=$(df --output=iavail /destination | tail -1 | tr -d '[:space:]')
test "$available_bytes" -gt "$copy_bytes"
test "$available_inodes" -gt "$used_inodes"
failure_step=archive
if tar --create --file=- --format=pax --directory=/source --one-file-system --sparse --xattrs --xattrs-include='*' --acls --numeric-owner . 2>/tmp/copy-source-errors \
    | tar --extract --file=- --directory=/destination --sparse --xattrs --xattrs-include='*' --acls --numeric-owner --same-owner --same-permissions --delay-directory-restore 2>/tmp/copy-destination-errors; then
    :
else
    archive_status=("${PIPESTATUS[@]}")
    printf '{"stage":"copy","step":"archive","source_exit_code":%s,"destination_exit_code":%s}' "${archive_status[0]}" "${archive_status[1]}" > "$failure_details"
    exit 1
fi
failure_step=restore-metadata
python3 -I - <<'PYTHON' 2>/tmp/copy-metadata-errors
import json
import os
import socket
import stat
import sys
import uuid

step = "restore-metadata"
socket_links = {}

def report_failure(error_type, error, traceback):
    with open("/tmp/copy-failure", "w") as output:
        json.dump({"stage": "copy", "step": step, "error": error_type.__name__,
                   "errno": getattr(error, "errno", None)}, output)

sys.excepthook = report_failure

def attributes(path):
    names = os.listxattr(path, follow_symlinks=False)
    if any(not name.startswith("user.") and name not in ("system.posix_acl_access", "system.posix_acl_default") for name in names):
        raise RuntimeError("unsupported visible xattr namespace")
    return {name: os.getxattr(path, name, follow_symlinks=False) for name in names}

def restore_socket(destination, metadata):
    inode = (metadata.st_dev, metadata.st_ino)
    if inode in socket_links:
        os.link(socket_links[inode], destination, follow_symlinks=False)
        return
    directory = os.open(os.path.dirname(destination), os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    temporary = ".mwc-shrink-socket-" + uuid.uuid4().hex
    created = False
    try:
        os.fchdir(directory)
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as offline:
            offline.bind(temporary)
            created = True
        os.link(temporary, destination, follow_symlinks=False)
    finally:
        if created:
            os.unlink(temporary, dir_fd=directory)
        os.chdir("/")
        os.close(directory)
    socket_links[inode] = destination

def restore(source, destination):
    global step
    step = "entry-type"
    metadata = os.lstat(source)
    if stat.S_ISSOCK(metadata.st_mode):
        step = "offline-socket"
        restore_socket(destination, metadata)
    copied = os.lstat(destination)
    if stat.S_IFMT(metadata.st_mode) != stat.S_IFMT(copied.st_mode):
        raise RuntimeError("entry type mismatch")
    if stat.S_ISDIR(metadata.st_mode):
        for name in sorted(os.listdir(source)):
            restore(os.path.join(source, name), os.path.join(destination, name))
    step = "ownership"
    if (metadata.st_uid, metadata.st_gid) != (copied.st_uid, copied.st_gid):
        os.chown(destination, metadata.st_uid, metadata.st_gid, follow_symlinks=False)
    step = "xattrs-policy"
    expected = attributes(source)
    existing = attributes(destination)
    step = "xattrs-remove"
    for name in existing.keys() - expected.keys():
        os.removexattr(destination, name, follow_symlinks=False)
    step = "mode"
    if not stat.S_ISLNK(metadata.st_mode):
        os.chmod(destination, stat.S_IMODE(metadata.st_mode), follow_symlinks=False)
    step = "xattrs-restore"
    for name, value in expected.items():
        os.setxattr(destination, name, value, follow_symlinks=False)
    step = "nanosecond-times"
    os.utime(destination, ns=(metadata.st_atime_ns, metadata.st_mtime_ns), follow_symlinks=False)

restore("/source", "/destination")
PYTHON
failure_step=sync
sync
printf '{"used_bytes":%s,"used_inodes":%s}' "$used_bytes" "$used_inodes" > /dev/termination-log
