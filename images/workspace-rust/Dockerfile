# linux/amd64 child of the attested Workspace Contract v1 base published from d0a6083.
FROM ghcr.io/memeloop-online/memeloop-workspace-control-workspace@sha256:93360d7ff4ab3d1c46e87eb156f3ba5c6367b0b5c816491a3f1d00e72aafd379 AS workspace-base

FROM workspace-base AS toolchain
RUN apt-get update \
    && apt-get install -y --no-install-recommends curl xz-utils \
    && rm -rf /var/lib/apt/lists/*
COPY rust.sha256 /tmp/rust.sha256
RUN set -eu; \
    mkdir /tmp/rust; cd /tmp/rust; \
    while read -r checksum archive; do \
      curl --fail --silent --show-error --location --retry 3 \
        --output "$archive" "https://static.rust-lang.org/dist/2026-08-20/$archive"; \
    done </tmp/rust.sha256; \
    sha256sum --strict --check /tmp/rust.sha256; \
    for archive in *.tar.xz; do \
      tar -xJf "$archive"; \
      "./${archive%.tar.xz}/install.sh" --prefix=/opt/mwc-rust --disable-ldconfig; \
    done

FROM workspace-base
RUN apt-get update \
    && apt-get install -y --no-install-recommends gcc libc6-dev git libssl-dev make pkg-config \
    && rm -rf /var/lib/apt/lists/*
COPY --from=toolchain /opt/mwc-rust/ /opt/mwc-rust/
ENV PATH="/opt/mwc-rust/bin:${PATH}"

RUN set -eu; \
    test -x /usr/local/bin/mwc-workspace-bootstrap; \
    test -x /usr/sbin/sshd; \
    test -x /usr/bin/ssh-keygen; \
    command -v jq; command -v install; \
    rustc --version | grep -F 'rustc 1.98.0 '; \
    cargo --version | grep -F 'cargo 1.98.0 '; \
    rustfmt --version; cargo clippy --version; \
    project="$(mktemp -d)"; trap 'rm -rf "$project"' EXIT; \
    chown workspace:workspace "$project"; \
    runuser -u workspace -- cargo new --quiet --vcs none --name mwc_smoke "$project/smoke"; \
    runuser -u workspace -- cargo run --quiet --offline --manifest-path "$project/smoke/Cargo.toml"
