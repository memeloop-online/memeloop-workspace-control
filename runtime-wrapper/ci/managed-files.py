#!/usr/bin/env python3
"""CI-only assertions against actual CRI/shim artifacts and mounted files."""
import json
import os
import pathlib
import stat
import sys

artifacts = pathlib.Path(sys.argv[1])
main = json.loads((artifacts / "workspace-wrapper.json").read_text())
sidecar = json.loads((artifacts / "ttyd-wrapper.json").read_text())
managed = {f["destination"]: f for f in main["managed_files"]}
peer = {f["destination"]: f for f in sidecar["managed_files"]}
assert set(managed) == {"/etc/hosts", "/etc/hostname", "/etc/resolv.conf"}
assert managed == peer, "same-sandbox containers must bind the same files"
proof = []
for destination, f in managed.items():
    original, redirected = pathlib.Path(f["original"]), pathlib.Path(f["redirected"])
    assert original.read_bytes() == redirected.read_bytes(), destination
    a, b = original.stat(), redirected.stat()
    assert (a.st_uid, a.st_gid, stat.S_IMODE(a.st_mode)) == (
        b.st_uid, b.st_gid, stat.S_IMODE(b.st_mode)
    ), destination
    assert b.st_dev == pathlib.Path(main["scratch"]["path"]).stat().st_dev
    attrs = {key: os.getxattr(original, key) for key in os.listxattr(original)}
    assert attrs == {key: os.getxattr(redirected, key) for key in os.listxattr(redirected)}
    proof.append({"destination": destination, "shared_source": str(redirected),
                  "initial_content_equal": True, "owner_mode_xattrs_equal": True,
                  "scratch_device": b.st_dev})

# CRI retains its original complete runtimeSpec; the shim bundle is what real
# runc consumes. Compare every field, allowing precisely the selected sources.
for name in ("workspace", "ttyd"):
    original = json.loads((artifacts / (name + "-inspect.json")).read_text())["info"]["runtimeSpec"]
    actual = json.loads((artifacts / (name + "-oci.json")).read_text())
    assert original["root"].get("readonly", False) == (name == "ttyd"), name
    assert actual["root"].get("readonly", False) == (name == "ttyd"), name
    if name == "ttyd":
        assert actual["process"]["noNewPrivileges"] is True
        assert "CAP_SYS_ADMIN" not in actual["process"]["capabilities"]["bounding"]
    for m in original["mounts"]:
        if m["destination"] in managed:
            assert m["source"] == managed[m["destination"]]["original"]
            m["source"] = managed[m["destination"]]["redirected"]
    assert original == actual, "OCI fields changed beyond managed file sources: " + name

(artifacts / "managed-files-proof.json").write_text(json.dumps(proof, indent=2))
