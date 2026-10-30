# CI-only. All VM devices are ordinary files in the one job-owned volume.
FROM harbor.k3s.onetwo.website/docker-io/library/debian:bookworm-slim@sha256:99c0b25c46d0214d5a2091092e7908907dcaecd6a962cde0ac700acee222b6f3
RUN rm -f /etc/apt/sources.list /etc/apt/sources.list.d/*.sources \
    && printf '%s\n' 'deb [check-valid-until=no] http://snapshot.debian.org/archive/debian/20261026T000000Z bookworm main' > /etc/apt/sources.list \
    && apt-get -o Acquire::http::Timeout=30 update \
    && DEBIAN_FRONTEND=noninteractive apt-get install --yes --no-install-recommends \
       ca-certificates curl jq qemu-system-x86 qemu-utils genisoimage skopeo coreutils \
    && rm -rf /var/lib/apt/lists/* /var/cache/apt/archives/*.deb \
    && mkdir /work /app && chown 65532:65532 /work
COPY tests/runtime-product-vm /app/vm
COPY tests/runtime_product_storage.sh /app/runtime_product_storage.sh
USER 65532:65532
WORKDIR /work
ENTRYPOINT ["/bin/bash", "/app/vm/smoke.sh"]
