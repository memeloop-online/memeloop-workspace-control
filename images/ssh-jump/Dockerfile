FROM debian:bookworm-slim

RUN apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get install --no-install-recommends -y \
        ca-certificates curl libnss-wrapper openssh-server \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 19000 --home /nonexistent --shell /usr/sbin/nologin mwc-auth \
    && ln -s "$(dpkg -L libnss-wrapper | grep '/libnss_wrapper.so$' | head -n 1)" \
        /usr/local/lib/libnss_wrapper.so \
    && mkdir -p /run/sshd /run/mwc /etc/ssh/host-keys

COPY sshd_config /etc/ssh/sshd_config
COPY mwc-authorized-keys /usr/local/bin/mwc-authorized-keys
COPY entrypoint.sh /usr/local/bin/mwc-ssh-jump-entrypoint

RUN chmod 0755 /usr/local/bin/mwc-authorized-keys /usr/local/bin/mwc-ssh-jump-entrypoint \
    && chmod 0644 /etc/ssh/sshd_config

EXPOSE 2222
ENTRYPOINT ["/usr/local/bin/mwc-ssh-jump-entrypoint"]
