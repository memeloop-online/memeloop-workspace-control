"""End only the shim listening on a disposable CI sandbox's recorded socket."""

import json
import os
import pathlib
import re
import select
import signal
import subprocess
import sys

bundle, sandbox, runtime = sys.argv[1:]
assert os.environ.get("GITHUB_ACTIONS") == "true"
assert re.fullmatch(r"[0-9a-f]{64}", sandbox)
root = pathlib.Path(bundle)
assert root == pathlib.Path("/var/lib/mwc-product-storage-ci/containerd-state/io.containerd.runtime.v2.task/k8s.io")
record = json.loads((root / sandbox / "bootstrap.json").read_text())
address = next(value for key, value in record.items() if key.lower() == "address")
socket = address.removeprefix("unix://")
assert socket.startswith(("/", "@")) and len(socket) > 12
listeners = subprocess.check_output(["ss", "-H", "-xlnp"], text=True)
pids = {
    int(pid)
    for line in listeners.splitlines()
    if socket in line.split()
    for pid in re.findall(r"pid=(\d+)", line)
}
assert len(pids) == 1, f"expected one shim listener, found {len(pids)}"
(pid,) = pids
proc = pathlib.Path("/proc", str(pid))
descriptor = os.pidfd_open(pid)
try:
    expected = {
        "runc": "containerd-shim-runc-v2",
        "mwc-native": "containerd-shim-runc-v2",
        "mwc-runsc": "containerd-shim-runsc-v1",
    }[runtime]
    assert pathlib.Path(os.readlink(proc / "exe")).name == expected
    args = (proc / "cmdline").read_bytes().decode().split("\0")
    assert args[args.index("-namespace") + 1] == "k8s.io"
    assert args[args.index("-id") + 1] == sandbox
    assert os.readlink(proc / "ns/mnt") == os.readlink("/proc/self/ns/mnt")
    signal.pidfd_send_signal(descriptor, signal.SIGKILL)
    poll = select.poll()
    poll.register(descriptor, select.POLLIN)
    assert poll.poll(10000), "fixture shim did not exit"
finally:
    os.close(descriptor)
print(json.dumps({"attested_shim": expected, "fixture_sandbox": sandbox, "process_exited": True}))
