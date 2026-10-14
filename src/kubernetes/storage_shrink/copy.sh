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
available_bytes=$(df -B1 --output=avail /destination | tail -1 | tr -d '[:space:]')
available_inodes=$(df --output=iavail /destination | tail -1 | tr -d '[:space:]')
test "$available_bytes" -gt "$copy_bytes"
test "$available_inodes" -gt "$used_inodes"
tar --create --file=- --format=pax --directory=/source --one-file-system --sparse --xattrs --xattrs-include='*' --acls --numeric-owner . 2>/tmp/copy-source-errors \
    | tar --extract --file=- --directory=/destination --sparse --xattrs --xattrs-include='*' --acls --numeric-owner --same-owner --same-permissions --delay-directory-restore 2>/tmp/copy-destination-errors
sync
printf '{"used_bytes":%s,"used_inodes":%s}' "$used_bytes" "$used_inodes" > /dev/termination-log
