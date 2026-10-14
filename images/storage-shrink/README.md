# Storage shrink helper

Build this dedicated helper only in the image CI pipeline. `BASE_IMAGE` defaults
to a reviewed digest-pinned Debian base (see "Base image verification" below).
Review the resulting image and configure its immutable digest as
`MWC_STORAGE_SHRINK_IMAGE` on the control plane. The workspace image is never
used as a fallback. No dependencies are installed and no network downloads
occur in a migration Pod.

## Base image verification

`BASE_IMAGE` pins `debian@sha256:a4672c0cb26fbdde88e38fa2dfb6c681942306680e41e4378b28770b6e79ee91`,
the `linux/amd64` child manifest digest of `debian:bookworm-slim`. On
2026-10-13 two independent registry mirrors of the official Docker Hub
`library/debian` repository returned the same digest for this tag, and the
amd64 manifest was re-fetched by digest with its SHA-256 content hash
recomputed to match. Direct Docker Hub API access was unavailable from the
review environment, so no direct Hub verification has succeeded yet; the final
acceptance source is the CI image build itself, where buildx pulls this exact
digest from the official `docker.io` registry and a mismatch fails the build.
Re-verify the amd64 digest through the official Docker Hub registry read-only
API (`/v2/library/debian/manifests/bookworm-slim`) before bumping the pin.

`ci-preflight.sh` runs inside the image in CI (against the local build in the
verify job and against the published digest in the publish job) and proves GNU
tar, Python 3 and GNU coreutils with working user xattr, POSIX ACL, sparse-file
and hardlink support before any migration may be enabled.

The helper includes Bash, GNU tar, Python 3 and coreutils. It receives no injected
workspace credentials and no service-account token. It copies a read-only source
claim to an exclusive destination claim, then verifies file SHA-256, POSIX metadata,
symlinks, visible supported xattrs and hardlink equivalence.

Only POSIX ACLs and `user.*` extended attributes are supported. The administrator
must explicitly set `MWC_STORAGE_SHRINK_XATTR_POLICY=posix-user` after verifying that
the platform does not use unsupported xattr namespaces. Visible unsupported
attributes cause verification to fail; `trusted.*` attributes cannot be certified
without elevated capabilities and must not be present under this policy. The
helper deliberately receives no `SYS_ADMIN` capability. This is not a generic
full-filesystem backup tool for arbitrary security/xattr configurations.

See `docs/storage-shrink.md` for the API, retained-volume confirmation and recovery.
