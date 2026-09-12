FROM debian:bookworm-slim AS payload

ARG GVISOR_RELEASE=release-20260831.0
ARG GVISOR_ARCHIVE_SHA256=014b3871a5c698c802fd7a03758e0dbf4c1683f9e3f8c743979ea66bbf6553a4

RUN apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends ca-certificates curl \
    && install -d -m 0755 /payload \
    && curl --fail --location --proto '=https' --tlsv1.2 \
      "https://github.com/google/gvisor/releases/download/${GVISOR_RELEASE}/gvisor-x86_64.tar.bz2" \
      --output /payload/gvisor-x86_64.tar.bz2 \
    && printf '%s  %s\n' "${GVISOR_ARCHIVE_SHA256}" /payload/gvisor-x86_64.tar.bz2 \
      | sha256sum --check --strict

FROM debian:bookworm-slim

RUN apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
      bash ca-certificates coreutils util-linux \
    && rm -rf /var/lib/apt/lists/*

COPY scripts/gvisor-job-entrypoint.sh /opt/mwc/gvisor-job-entrypoint.sh
COPY scripts/gvisor-host-transaction.sh /opt/mwc/gvisor-host-transaction.sh
COPY scripts/gvisor-node-install.sh /opt/mwc/gvisor-node-install.sh
COPY scripts/gvisor-node-preflight.sh /opt/mwc/gvisor-node-preflight.sh
COPY scripts/gvisor-node-rollback.sh /opt/mwc/gvisor-node-rollback.sh
COPY images/gvisor-node-installer/mwc-gvisor-recovery@.service /opt/mwc/mwc-gvisor-recovery@.service
COPY --from=payload /payload/gvisor-x86_64.tar.bz2 /opt/mwc/gvisor-x86_64.tar.bz2

RUN chmod 0755 /opt/mwc/*.sh

ENTRYPOINT ["/opt/mwc/gvisor-job-entrypoint.sh"]
