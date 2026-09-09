# syntax=docker/dockerfile:1
FROM debian:bookworm-slim AS build

ARG TTYD_COMMIT=40e79c706be14029b391f369bee6613c31667abb
ARG TTYD_TARBALL_SHA256=a04cead2cf8fccc8a73d03c53d1617680322c9655e031f0105395af100d296bd
ARG LWS_COMMIT=4415e84c095857629863804e941b9e1c2e9347ef
ARG LWS_TARBALL_SHA256=6fd33527b410a37ebc91bb64ca51bdabab12b076bc99d153d7c5dd405e4bdf90

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential ca-certificates cmake curl libjson-c-dev libssl-dev \
        libuv1-dev zlib1g-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /src

# Use upstream libwebsockets with OpenSSL explicitly selected.  The upstream
# ttyd Linux binary instead uses Mbed TLS 2.x and must not provide this mTLS
# boundary.
RUN curl --fail --location --silent --show-error --retry 3 \
        --output lws.tar.gz "https://codeload.github.com/warmcat/libwebsockets/tar.gz/${LWS_COMMIT}" \
    && echo "${LWS_TARBALL_SHA256}  lws.tar.gz" | sha256sum --check --strict \
    && mkdir lws \
    && tar --extract --file lws.tar.gz --gzip --strip-components=1 --directory lws \
    && cmake -S lws -B lws-build \
        -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX=/usr/local \
        -DLWS_WITH_EVLIB_PLUGINS=OFF -DLWS_WITH_LIBUV=ON -DLWS_WITH_MBEDTLS=OFF \
        -DLWS_WITH_SHARED=OFF \
        -DLWS_WITH_SSL=ON -DLWS_WITHOUT_TESTAPPS=ON \
    && cmake --build lws-build --parallel \
    && cmake --install lws-build \
    && grep --quiet '^LWS_WITH_EVLIB_PLUGINS:BOOL=OFF$' lws-build/CMakeCache.txt \
    && grep --quiet '^LWS_WITH_SSL:BOOL=ON$' lws-build/CMakeCache.txt \
    && grep --quiet '^LWS_WITH_MBEDTLS:BOOL=OFF$' lws-build/CMakeCache.txt

RUN curl --fail --location --silent --show-error --retry 3 \
        --output ttyd.tar.gz "https://codeload.github.com/tsl0922/ttyd/tar.gz/${TTYD_COMMIT}" \
    && echo "${TTYD_TARBALL_SHA256}  ttyd.tar.gz" | sha256sum --check --strict \
    && mkdir ttyd \
    && tar --extract --file ttyd.tar.gz --gzip --strip-components=1 --directory ttyd \
    && cmake -S ttyd -B ttyd-build \
        -DCMAKE_BUILD_TYPE=Release -DCMAKE_PREFIX_PATH=/usr/local \
    && cmake --build ttyd-build --parallel \
    && ldd ttyd-build/ttyd | grep --quiet 'libssl.so'

FROM debian:bookworm-slim

ARG TTYD_COMMIT=40e79c706be14029b391f369bee6613c31667abb
LABEL org.opencontainers.image.source="https://github.com/tsl0922/ttyd" \
      org.opencontainers.image.revision="${TTYD_COMMIT}"

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        libjson-c5 libssl3 libuv1 openssh-client zlib1g \
    && rm -rf /var/lib/apt/lists/*

COPY --from=build /src/ttyd-build/ttyd /usr/bin/ttyd

USER root
EXPOSE 7681
WORKDIR /root
ENTRYPOINT ["/usr/bin/ttyd"]
CMD ["-W", "bash"]
