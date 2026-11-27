ARG BASE_IMAGE=ghcr.io/topolvm/topolvm-with-sidecar@sha256:70548dbe0c6addcccf79a557f29e95db2e6dc2cba2102988c91f30086004d0fc
FROM ${BASE_IMAGE}

# Copy into an independent layer so the base symlink is replaced, not followed.
COPY --link --chmod=0755 topolvm-node /topolvm-node
