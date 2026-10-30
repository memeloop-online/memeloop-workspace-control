#!/usr/bin/python3
"""Disposable CI observer: kill the wrapper at a real, externally observed edge."""
import json
import os
import pathlib
import re
import signal
import subprocess
import sys
import time

stage, wrapper, *args = sys.argv[1:]
if "create" not in args:
    os.execv(wrapper, [wrapper, *args])
cid = args[-1]
if stage not in ("prepared", "redirected", "runtime-created") or not re.fullmatch("[a-f0-9]{64}", cid):
    raise SystemExit("unsupported CI hardkill invocation")
base = pathlib.Path("/var/lib/mwc-runtime-prototype")
out = pathlib.Path(os.environ["GITHUB_WORKSPACE"], "runtime-wrapper/artifacts", "hardkill-" + stage)
out.mkdir(exist_ok=True)
(out / "id.txt").write_text(cid + "\n")
record = base / "state" / (cid + ".json")
pending = record.with_suffix(".json.new")


def read_record(path):
    try:
        return json.loads(path.read_text())
    except (FileNotFoundError, json.JSONDecodeError):
        return None


def matching_mount(rec, text, original=False):
    for line in text.splitlines():
        left, _, right = line.partition(" - ")
        fields = left.split()
        if len(fields) > 4 and fields[4] == rec["rootfs"]:
            if original:
                return int(fields[0]) == rec["original"]["id"]
            return ",upperdir=" + rec["upper"] + "," in "," + right.split()[-1] + ","
    return False


# Follow Go threads, then detach on exec of dmsetup/the actual runtime. Exact
# strace skips the initial wrapper exec, so Sentry/Gofer/init remain untraced.
# The delay only opens an observation window; it never forges syscall success.
trace = subprocess.Popen([
    "/usr/bin/strace", "-ff", "-b", "execve", "-qq", "-o", str(out / "syscalls"),
    "-e", "trace=rename,renameat,renameat2,mount,umount2,execve,wait4,waitid",
    "-e", "inject=rename,renameat,renameat2:delay_enter=250ms", wrapper, *args,
], close_fds=False)
wrapper_pid = None
observed = None
deadline = time.monotonic() + 45
while time.monotonic() < deadline and trace.poll() is None:
    if wrapper_pid is None:
        try:
            children = pathlib.Path(f"/proc/{trace.pid}/task/{trace.pid}/children").read_text().split()
            for pid in children:
                try:
                    if os.readlink(f"/proc/{pid}/exe") == wrapper:
                        wrapper_pid = int(pid)
                        break
                except FileNotFoundError:
                    pass
        except FileNotFoundError:
            pass
    committed, new = read_record(record), read_record(pending)
    text = pathlib.Path("/proc/self/mountinfo").read_text()
    if wrapper_pid and committed and new:
        if stage == "prepared":
            ready = committed["phase"] == new["phase"] == "prepared" and bool(new["mount_data"]) and matching_mount(new, text, original=True)
        else:
            expected = "redirected" if stage == "redirected" else "created"
            ready = new["phase"] == expected and matching_mount(new, text)
        if ready:
            observed = {"stage": stage, "container_id": cid, "wrapper_pid": wrapper_pid,
                        "committed": committed, "pending_fsynced_record": new,
                        "observed_unix_ns": time.time_ns()}
            observed["rootfs_device"] = os.stat(new["rootfs"]).st_dev
            # Kill immediately while the traced rename is held, before scanning
            # references. The real shim is still waiting for this observer.
            os.kill(wrapper_pid, signal.SIGKILL)
            (out / "before-kill-mountinfo.txt").write_text(text)
            break
    time.sleep(0.002)
if observed is None:
    (out / "injection-failed.txt").write_text(f"trace_status={trace.poll()} wrapper_pid={wrapper_pid}\n")
    if wrapper_pid:
        os.kill(wrapper_pid, signal.SIGKILL)
    trace.wait(timeout=10)
    raise SystemExit("hardkill observation edge was not reached")
status = trace.wait(timeout=10)
observed["tracer_exit_code"] = status
(out / "observed.json").write_text(json.dumps(observed, indent=2))
(out / "after-kill-mountinfo.txt").write_text(pathlib.Path("/proc/self/mountinfo").read_text())
for name, path in (("record", record), ("pending", pending), ("oci", pathlib.Path(observed["committed"]["bundle"], "config.json"))):
    if path.exists():
        (out / ("after-kill-" + name + ".json")).write_bytes(path.read_bytes())
cfg = json.loads(pathlib.Path("/etc/mwc-runsc-wrapper.json" if "runsc" in wrapper else "/etc/mwc-runtime-wrapper.json").read_text())
root = None
for i, arg in enumerate(args[:args.index("create")]):
    if arg.startswith("--root="):
        root = arg.split("=", 1)[1]
    elif arg == "--root":
        root = args[i + 1]
if not root:
    raise SystemExit("actual shim runtime root was not provided")
state = subprocess.run([cfg["real_runtime"], "--root", root, "state", cid], capture_output=True, timeout=5)
(out / "runtime-state.json").write_text(json.dumps({"status": state.returncode, "stdout": state.stdout.decode(errors="replace"), "stderr": state.stderr.decode(errors="replace")}, indent=2))
(out / "runtime-root.txt").write_text(root + "\n")
subprocess.run([sys.executable, str(pathlib.Path(__file__).with_name("hardkill-references.py")),
                str(out), "after-kill"], check=True)
# The binary invocation failed by SIGKILL. Let the real shim execute its normal
# failure cleanup; the observer never unmounts, deletes an upper or kills tasks.
raise SystemExit(137)
