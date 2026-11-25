ARG BASE_IMAGE=ghcr.io/topolvm/topolvm-with-sidecar@sha256:70548dbe0c6addcccf79a557f29e95db2e6dc2cba2102988c91f30086004d0fc
FROM ${BASE_IMAGE}

COPY topolvm-node /tmp/topolvm-node-patched
RUN test "$(readlink /topolvm-node)" = hypertopolvm \
    && rm /topolvm-node \
    && install -m 0755 /tmp/topolvm-node-patched /topolvm-node \
    && rm /tmp/topolvm-node-patched
