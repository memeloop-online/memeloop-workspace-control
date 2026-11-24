"""Report holders of the exact disposable BuildKit cache inode, without touching it."""

import json
import os
import pathlib
import sys

lock = pathlib.Path(sys.argv[1])
info = lock.stat()
identity = (os.major(info.st_dev), os.minor(info.st_dev), info.st_ino)
holders = []
for line in pathlib.Path("/proc/locks").read_text().splitlines():
    fields = line.split()
    if fields[1] != "FLOCK":
        continue
    major, minor, inode = fields[5].split(":")
    if identity != (int(major, 16), int(minor, 16), int(inode)):
        continue
    pid = int(fields[4])
    entry = {"pid": pid}
    proc = pathlib.Path("/proc", str(pid))
    try:
        entry["command"] = (proc / "comm").read_text().strip()
        entry["cgroup"] = (proc / "cgroup").read_text().splitlines()
        entry["start_ticks"] = int((proc / "stat").read_text().rsplit(") ", 1)[1].split()[19])
    except FileNotFoundError:
        entry["exited_during_read"] = True
    holders.append(entry)
print(json.dumps({"device": identity[:2], "inode": identity[2], "holders": holders}))
