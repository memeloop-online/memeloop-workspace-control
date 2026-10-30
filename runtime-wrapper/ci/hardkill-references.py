#!/usr/bin/python3
"""Read-only CI evidence for one exact injected ID, never cleanup authority."""
import json
import os
import pathlib
import sys

out, point = pathlib.Path(sys.argv[1]), sys.argv[2]
observed = json.loads((out / "observed.json").read_text())
cid = observed["container_id"]
scratch_dev = os.stat(observed["committed"]["scratch"]["path"]).st_dev
devices = {scratch_dev, observed["rootfs_device"]}
refs = []
for proc in pathlib.Path("/proc").iterdir():
    if not proc.name.isdecimal():
        continue
    try:
        cmd = proc.joinpath("cmdline").read_bytes().replace(b"\0", b" ").decode(errors="replace")
        relevant = [line for line in proc.joinpath("mountinfo").read_text().splitlines() if cid in line]
        fds = []
        for fd in proc.joinpath("fd").iterdir():
            try:
                dev = fd.stat().st_dev
                if dev in devices:
                    fds.append({"fd": fd.name, "device": dev, "target": os.readlink(fd)})
            except OSError:
                pass
        if relevant or fds or cid in cmd:
            refs.append({"pid": int(proc.name), "command": cmd, "mounts": relevant, "fds": fds})
    except OSError:
        pass
(out / (point + "-process-references.json")).write_text(json.dumps(refs, indent=2))
