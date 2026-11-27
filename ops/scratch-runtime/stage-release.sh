#!/bin/bash
# Package only the files consumed by the successful, disposable CSI run.
set -euo pipefail
: "${RUNNER_TEMP:?}" "${GITHUB_WORKSPACE:?}" "${MWC_PRODUCT_STORAGE_INPUTS:?}"
: "${CONTAINERD_BASE_SHA:?}" "${CONTAINERD_CLEANUP_SHA:?}"

src="$GITHUB_WORKSPACE/ops/scratch-runtime"
compiled="$GITHUB_WORKSPACE/input-bundle"
bin="$RUNNER_TEMP/product-storage-bin"
stage="$RUNNER_TEMP/scratch-runtime-release"
oci="$RUNNER_TEMP/scratch-runtime-oci"
mkdir -p "$stage/"{bin,boot,licenses/mwc,licenses/containerd} "$oci"

for name in mwc-runtime-wrapper mwc-runsc-wrapper containerd-shim-cleanup; do
    cmp "$bin/$name" "$compiled/bin/$name"
    install -m 0755 "$bin/$name" "$stage/bin/$name"
done
install -m 0755 "$src/mwc-scratch-mounts.sh" "$stage/boot/"
install -m 0644 "$src/"{mwc-scratch-mounts.service,30-mwc-scratch-mounts.conf} "$stage/boot/"
install -m 0644 "$GITHUB_WORKSPACE/"{LICENSE,NOTICE} "$stage/licenses/mwc/"
install -m 0644 "$compiled/containerd-licenses/"{LICENSE,NOTICE} "$stage/licenses/containerd/"

# The gVisor archive is the whole upstream release used by the shim tests.
(
    cd "$MWC_PRODUCT_STORAGE_INPUTS"
    grep -E '  (k3s|gvisor\.tar\.bz2)$' SHA256SUMS | sha256sum --strict -c -
)
install -m 0644 "$MWC_PRODUCT_STORAGE_INPUTS/gvisor.tar.bz2" "$stage/"
jq --arg fork "$CONTAINERD_BASE_SHA" --arg patch "$CONTAINERD_CLEANUP_SHA" \
   --arg k3s "$(sha256sum "$MWC_PRODUCT_STORAGE_INPUTS/k3s" | cut -d' ' -f1)" \
   '. + {platform:"linux/amd64",containerd_fork_sha:$fork,containerd_cleanup_patch_sha:$patch,k3s_binary_sha256:$k3s}' \
   "$compiled/source-shas.json" > "$stage/identity.json"
(
    cd "$stage"
    sha256sum bin/* boot/* gvisor.tar.bz2 identity.json licenses/*/* > SHA256SUMS
    tar --sort=name --owner=0 --group=0 --numeric-owner -czf "$oci/mwc-scratch-runtime-linux-amd64.tar.gz" \
        bin boot gvisor.tar.bz2 identity.json licenses SHA256SUMS
)
