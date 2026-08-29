FROM rust:1.98-bookworm AS builder
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY wit ./wit
COPY web/dist ./web/dist
COPY images/workspace-base/mwc-workspace-bootstrap ./images/workspace-base/mwc-workspace-bootstrap
RUN cargo build --locked --release

FROM debian:bookworm-slim
RUN apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends ca-certificates \
    && groupadd --gid 19000 mwc \
    && useradd --uid 19000 --gid 19000 --no-create-home --home-dir /var/lib/mwc mwc \
    && install -d -m 0750 -o mwc -g mwc /var/lib/mwc \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /build/target/release/memeloop-workspace-control /usr/local/bin/memeloop-workspace-control
USER 19000:19000
EXPOSE 8080 8081
ENTRYPOINT ["/usr/local/bin/memeloop-workspace-control"]
