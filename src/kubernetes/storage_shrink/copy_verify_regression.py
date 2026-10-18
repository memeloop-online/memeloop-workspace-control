import hashlib
import json
import os
from pathlib import Path
import shutil
import socket
import stat
import struct
import subprocess
import tempfile
import unittest


def acl(default=False):
    entries = [(1, 7, 0xFFFFFFFF), (2, 4, os.getuid()), (4, 5, 0xFFFFFFFF),
               (16, 5, 0xFFFFFFFF), (32, 0 if default else 1, 0xFFFFFFFF)]
    return struct.pack("<I", 2) + b"".join(
        struct.pack("<HHI", tag, permissions, identity)
        for tag, permissions, identity in entries
    )


class CopyVerifyRegression(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="mwc-shrink-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.source = self.root / "source"
        self.destination = self.root / "destination"
        self.scratch = self.root / "scratch"
        self.scratch.mkdir()
        self.termination = self.root / "termination.json"
        self.runtime = self.source / "build-cache/runtime"
        (self.runtime / "buildkit").mkdir(parents=True)
        self.destination.mkdir(mode=0o2770)
        os.chmod(self.destination, 0o2770)
        self.data = self.source / "data"
        self.data.write_bytes(b"unchanged payload")
        os.link(self.data, self.source / "hardlink")
        (self.source / "symlink").symlink_to("data")
        os.mkfifo(self.source / "fifo", 0o640)
        with (self.source / "sparse").open("wb") as sparse:
            sparse.truncate(8 * 1024 * 1024)
        os.setxattr(self.data, "user.shrink-regression", b"preserve-me")
        os.setxattr(self.data, "system.posix_acl_access", acl())
        (self.source / "legitimate-sgid").mkdir()
        os.chmod(self.source / "legitimate-sgid", 0o2750)
        os.chmod(self.data, 0o6751)
        self.socket = self.runtime / "buildkit/otel-grpc.sock"
        self.make_socket(self.socket)
        os.chmod(self.socket, 0o666)
        os.chmod(self.runtime, 0o700)
        os.setxattr(self.runtime, "user.shrink-directory", b"preserve-directory")
        os.setxattr(self.runtime / "buildkit", "system.posix_acl_default", acl(True))
        self.long_socket = self.source / ("long-directory-" * 10) / ("socket-" * 22)
        self.long_socket.parent.mkdir()
        self.make_socket(self.long_socket)
        for entry in sorted(self.source.rglob("*"), reverse=True) + [self.source]:
            os.utime(entry, ns=(1700000000123456789, 1700000000987654321),
                     follow_symlinks=False)
        self.original = self.snapshot()

    def make_socket(self, path):
        previous = os.open(".", os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.chdir(path.parent)
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as offline:
                offline.bind("short-socket")
            os.rename("short-socket", path.name)
        finally:
            os.fchdir(previous)
            os.close(previous)

    def snapshot(self):
        result = {}
        for entry in [self.source] + sorted(self.source.rglob("*")):
            metadata = entry.lstat()
            result[str(entry.relative_to(self.source))] = (
                metadata.st_mode, metadata.st_uid, metadata.st_gid,
                metadata.st_mtime_ns, metadata.st_nlink,
                {name: os.getxattr(entry, name, follow_symlinks=False)
                 for name in os.listxattr(entry, follow_symlinks=False)},
                hashlib.sha256(entry.read_bytes()).digest()
                if stat.S_ISREG(metadata.st_mode) else None,
            )
        return result

    def run_stage(self, stage, script=None, success=True):
        script = script or os.environ[f"SHRINK_{stage.upper()}_SCRIPT"]
        script = (script.replace("/tmp/", str(self.scratch) + "/")
                  .replace("/source", str(self.source))
                  .replace("/destination", str(self.destination))
                  .replace("/dev/termination-log", str(self.termination)))
        result = subprocess.run(
            ["/bin/bash", "-c", script], capture_output=True, text=True,
            env={**os.environ, "TARGET_BYTES": str(1 << 62)}, timeout=60,
        )
        self.assertEqual(result.returncode == 0, success, result.stderr)
        self.assertEqual(self.snapshot(), self.original, "source changed")
        return json.loads(self.termination.read_text())

    def test_socket_and_metadata_survive_inherited_sgid_and_acl(self):
        drift = f'''python3 -I - <<'DRIFT'
import os
os.chmod("/destination/build-cache/runtime", 0o2700)
os.setxattr("/destination/build-cache/runtime", "user.unwanted", b"remove-me")
os.setxattr("/destination/build-cache/runtime", "system.posix_acl_default", {acl(True)!r})
DRIFT
failure_step=restore-metadata'''
        script = os.environ["SHRINK_COPY_SCRIPT"].replace(
            "failure_step=restore-metadata", drift)
        self.run_stage("copy", script)
        evidence = self.run_stage("verify")
        self.assertGreater(evidence["used_inodes"], 1)
        copied_socket = self.destination / self.socket.relative_to(self.source)
        metadata = copied_socket.lstat()
        self.assertTrue(stat.S_ISSOCK(metadata.st_mode))
        self.assertEqual(stat.S_IMODE(metadata.st_mode), 0o666)
        self.assertTrue(stat.S_ISSOCK(
            (self.destination / self.long_socket.relative_to(self.source)).lstat().st_mode))
        self.assertEqual(stat.S_IMODE(
            (self.destination / "build-cache/runtime").stat().st_mode), 0o700)
        self.assertEqual(stat.S_IMODE(
            (self.destination / "legitimate-sgid").stat().st_mode), 0o2750)
        self.assertLess((self.destination / "sparse").stat().st_blocks * 512,
                        (self.destination / "sparse").stat().st_size)

    def test_hardlinked_sockets_are_not_split(self):
        os.link(self.socket, self.runtime / "buildkit/socket-hardlink")
        self.original = self.snapshot()
        self.run_stage("copy")
        self.run_stage("verify")

    def test_socket_creation_failure_is_closed_and_path_free(self):
        injection = '''import socket
def reject_socket(*args, **kwargs):
    raise PermissionError(13, "sensitive-detail", "/source/private-name")
socket.socket = reject_socket'''
        script = os.environ["SHRINK_COPY_SCRIPT"].replace("import socket", injection)
        failure = self.run_stage("copy", script, success=False)
        self.assertEqual(failure, {"stage": "copy", "step": "offline-socket",
                                   "error": "PermissionError", "errno": 13})
        self.assertNotIn("private-name", self.termination.read_text())
        self.assertNotIn("sensitive-detail", self.termination.read_text())

    def test_archive_failure_reports_both_exit_codes(self):
        script = os.environ["SHRINK_COPY_SCRIPT"].replace(
            "tar --create", "tar --invalid-shrink-option --create", 1)
        failure = self.run_stage("copy", script, success=False)
        self.assertEqual(failure["step"], "archive")
        self.assertNotEqual(failure["source_exit_code"], 0)
        self.assertNotEqual(failure["destination_exit_code"], 0)

    def test_verifier_still_rejects_loss_and_corruption(self):
        cases = ["socket-missing", "socket-type", "sgid", "mtime", "content",
                 "xattr", "hardlink"]
        for case in cases:
            with self.subTest(case=case):
                shutil.rmtree(self.destination)
                self.destination.mkdir()
                self.run_stage("copy")
                copied_socket = self.destination / self.socket.relative_to(self.source)
                if case in ("socket-missing", "socket-type"):
                    parent = copied_socket.parent.stat()
                    copied_socket.unlink()
                    if case == "socket-type":
                        copied_socket.write_bytes(b"")
                        os.chmod(copied_socket, 0o666)
                    os.utime(copied_socket.parent,
                             ns=(parent.st_atime_ns, parent.st_mtime_ns))
                elif case == "sgid":
                    os.chmod(self.destination / "build-cache/runtime", 0o2700)
                elif case == "mtime":
                    os.utime(self.destination / "data", ns=(1, 2))
                elif case == "content":
                    copied_data = self.destination / "data"
                    metadata = copied_data.stat()
                    copied_data.write_bytes(b"corrupted payload")
                    os.chmod(copied_data, stat.S_IMODE(metadata.st_mode))
                    os.utime(copied_data, ns=(metadata.st_atime_ns, metadata.st_mtime_ns))
                elif case == "xattr":
                    os.setxattr(self.destination / "data", "user.shrink-regression", b"wrong")
                else:
                    root_metadata = self.destination.stat()
                    hardlink = self.destination / "hardlink"
                    hardlink.unlink()
                    shutil.copy2(self.destination / "data", hardlink)
                    os.utime(self.destination,
                             ns=(root_metadata.st_atime_ns, root_metadata.st_mtime_ns))
                failure = self.run_stage("verify", success=False)
                self.assertEqual(failure["stage"], "verify")
                self.assertEqual(failure["error"], "AssertionError")
                self.assertNotIn(str(self.source), self.termination.read_text())


unittest.main(argv=["storage-shrink-regression"])
