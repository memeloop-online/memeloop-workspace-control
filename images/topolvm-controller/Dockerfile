ARG BASE_IMAGE=harbor.k3s.onetwo.website/ghcr-io/topolvm/topolvm-with-sidecar@sha256:70548dbe0c6addcccf79a557f29e95db2e6dc2cba2102988c91f30086004d0fc
FROM ${BASE_IMAGE}

ARG SOURCE_COMMIT
ARG PATCH_SHA256

LABEL org.opencontainers.image.source="https://github.com/topolvm/topolvm" \
      org.opencontainers.image.version="v0.41.1" \
      org.opencontainers.image.revision="${SOURCE_COMMIT}" \
      io.memeloop.topolvm.patch-sha256="${PATCH_SHA256}"

COPY topolvm-controller /tmp/topolvm-controller-patched
RUN test -L /topolvm-controller \
    && test "$(readlink /topolvm-controller)" = hypertopolvm \
    && rm /topolvm-controller \
    && install -m 0755 /tmp/topolvm-controller-patched /topolvm-controller \
    && rm /tmp/topolvm-controller-patched
