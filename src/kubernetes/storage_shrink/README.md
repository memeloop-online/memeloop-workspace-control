# Offline copy invariants

The source claim and mount remain read-only. GNU tar preserves ordinary files,
sparse allocation, hardlinks, symlinks, ACLs and supported xattrs, but silently
omits UNIX sockets. After extraction, the copy helper binds and closes an AF_UNIX
socket on the destination for each source socket inode. This preserves an offline
filesystem node, not a listener, connection, queued data or live process state.
A short temporary name in the destination directory avoids the AF_UNIX pathname
length limit; hardlinked socket names share the recreated inode.

The same post-order traversal restores numeric ownership, exact permission bits
(including clearing unwanted inherited SGID), supported xattrs/ACLs and nanosecond
modification times. Directories are restored after their children so socket
creation cannot leave directory timestamps changed. Source metadata is never
modified. Socket creation or metadata restoration failures fail the copy; no
source entry is excluded and verification retains all existing checks.

Copy/verify failures write bounded JSON to `/dev/termination-log`: stage, step,
exception class and errno, or archive producer/consumer exit codes. Paths, xattr
values and raw stderr are not copied into this evidence. This does not change
control-plane failure parsing or operational recovery behavior.

## CI only

The existing Rust unit suite invokes `copy_verify_regression.py`, exercising the
actual scripts against disposable fixtures, including socket omission, long
socket paths, socket hardlinks, inherited SGID, nanosecond timestamps, sparse
files, ACLs, xattrs, hardlinks, symlinks, FIFOs and sanitized failure evidence.
It also requires verification to reject deleted/retyped sockets, unexpected
SGID, mtime/content/xattr drift and split hardlinks. No dependencies are added.

Run in repository CI, not the shared workspace:

```sh
cargo test --locked --all-features copy_and_verify_preserve_offline_sockets_and_exact_metadata -- --nocapture
cargo test --locked --all-features --no-fail-fast
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
```

The existing helper-image preflight must also pass before release. These scripts
are embedded in the control-plane binary; rebuilding only the helper image does
not deploy a script change. Review and rebuild the control plane, and retain the
immutable approved helper image policy.
