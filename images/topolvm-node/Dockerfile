ARG BASE_IMAGE=ghcr.io/topolvm/topolvm-with-sidecar@sha256:70548dbe0c6addcccf79a557f29e95db2e6dc2cba2102988c91f30086004d0fc
FROM ${BASE_IMAGE}

# Remove the base symlink before writing; the build input is never a layer.
RUN --mount=type=bind,source=topolvm-node,target=/tmp/topolvm-node-patched \
    rm /topolvm-node && install -m 0755 /tmp/topolvm-node-patched /topolvm-node
