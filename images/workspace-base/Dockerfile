FROM debian:bookworm-slim

RUN apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        ca-certificates jq openssh-client openssh-server tini \
    && useradd --create-home --shell /bin/bash workspace \
    && mkdir -p /run/sshd /workspace \
    && chown workspace:workspace /workspace \
    && rm -rf /var/lib/apt/lists/*

COPY mwc-workspace-bootstrap /usr/local/bin/mwc-workspace-bootstrap
RUN chmod 0755 /usr/local/bin/mwc-workspace-bootstrap

EXPOSE 2222
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/mwc-workspace-bootstrap"]
CMD ["serve"]
