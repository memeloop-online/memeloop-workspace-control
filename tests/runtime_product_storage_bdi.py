"""Read BDI policy for the private CI node; never write kernel settings."""

import json
import os
import sys
from pathlib import Path


BLOCK = Path("/sys/dev/block")
BDI = Path("/sys/devices/virtual/bdi")
VM = Path("/proc/sys/vm")
KNOBS = ("strict_limit", "min_ratio", "min_ratio_fine", "max_ratio", "max_ratio_fine")
VM_KNOBS = (
    "dirty_background_bytes", "dirty_background_ratio", "dirty_bytes",
    "dirty_ratio", "dirty_expire_centisecs", "dirty_writeback_centisecs",
)


def read(path):
    return path.read_text().strip()


def bdi_state(device):
    block = BLOCK / device
    assert read(block / "dev") == device, block
    path = (block / "bdi").resolve(strict=True)
    assert path == BDI / device, path
    return {
        "device": device,
        "dm_uuid": read(block / "dm/uuid"),
        "bdi": str(path),
        "knobs": {knob: int(read(path / knob)) for knob in KNOBS},
    }


def xfs_publish(path):
    mounts = [
        fields for line in Path("/proc/self/mountinfo").read_text().splitlines()
        if (fields := line.split())[4] == str(path)
    ]
    assert len(mounts) == 1, (path, mounts)
    mount = mounts[0]
    assert mount[3] == "/" and mount[mount.index("-") + 1] == "xfs", mount
    dev = path.stat().st_dev
    device = f"{os.major(dev)}:{os.minor(dev)}"
    assert mount[2] == device, mount
    return device


def policy_state(vg_uuid, pool_uuid, publishes):
    pool_devices = []
    for block in BLOCK.iterdir():
        try:
            if read(block / "dm/uuid") == pool_uuid:
                pool_devices.append(block.name)
        except FileNotFoundError:
            continue
    assert len(pool_devices) == 1, pool_devices
    pool = bdi_state(pool_devices[0])
    assert pool["dm_uuid"] == pool_uuid, pool
    state = {"scope": {"pool": pool, "vm": {knob: int(read(VM / knob)) for knob in VM_KNOBS}}}
    if publishes:
        scratch_path, home_path = map(Path, publishes)
        scratch = bdi_state(xfs_publish(scratch_path))
        home = bdi_state(xfs_publish(home_path))
        vg_prefix = "LVM-" + vg_uuid
        assert scratch["dm_uuid"].startswith(vg_prefix) and len(scratch["dm_uuid"]) == 68, scratch
        assert home["dm_uuid"].startswith("LVM-") and not home["dm_uuid"].startswith(vg_prefix), home
        slaves = list((BLOCK / scratch["device"] / "slaves").iterdir())
        assert len(slaves) == 1 and read(slaves[0] / "dev") == pool["device"], slaves
        assert len({scratch["device"], home["device"], pool["device"]}) == 3
        state.update(scratch=scratch, home=home)
    return state


if __name__ == "__main__":
    vg_uuid, pool_uuid, *publishes = sys.argv[1:]
    print(json.dumps(policy_state(vg_uuid, pool_uuid, publishes), indent=2))
