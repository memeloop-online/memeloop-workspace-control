FROM kalilinux/kali-rolling@sha256:49d174981a2a211e76acdde22a2e4978ea19bc0433487292cd9ad865da24ee42

ARG KALI_APT_MIRROR=http://mirrors.ustc.edu.cn/kali
LABEL org.opencontainers.image.title="MWC Kali browser desktop" \
      org.opencontainers.image.description="Kali rolling XFCE workspace with loopback-only TigerVNC and noVNC" \
      org.opencontainers.image.base.name="kalilinux/kali-rolling@sha256:49d174981a2a211e76acdde22a2e4978ea19bc0433487292cd9ad865da24ee42"

RUN rm -f /etc/apt/sources.list.d/* \
    && printf '%s\n' "deb ${KALI_APT_MIRROR} kali-rolling main contrib non-free non-free-firmware" \
        > /etc/apt/sources.list \
    && apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        ca-certificates dbus-x11 jq novnc openssh-client openssh-server \
        passwd tigervnc-standalone-server tini util-linux websockify xfce4 xfce4-terminal \
        xauth xdg-user-dirs \
    && useradd --create-home --shell /bin/bash workspace \
    && passwd --delete workspace \
    && mkdir -p /run/sshd /workspace \
    && chown workspace:workspace /workspace \
    && rm -rf /var/lib/apt/lists/*

COPY images/workspace-base/mwc-workspace-bootstrap /usr/local/bin/mwc-workspace-bootstrap
COPY images/kali-browser-desktop/mwc-workspace-desktop /usr/local/bin/mwc-workspace-desktop
COPY images/kali-browser-desktop/bwrap-gvisor-compat /usr/local/lib/mwc-gvisor-compat/bwrap
COPY images/kali-browser-desktop/index.html /usr/share/novnc/index.html
RUN chmod 0755 /usr/local/bin/mwc-workspace-bootstrap \
        /usr/local/bin/mwc-workspace-desktop \
        /usr/local/lib/mwc-gvisor-compat/bwrap

EXPOSE 2222 6080
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/mwc-workspace-bootstrap"]
CMD ["serve"]
