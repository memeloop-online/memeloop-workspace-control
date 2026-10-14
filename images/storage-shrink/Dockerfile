ARG BASE_IMAGE=debian@sha256:a4672c0cb26fbdde88e38fa2dfb6c681942306680e41e4378b28770b6e79ee91
FROM ${BASE_IMAGE}

USER root
RUN apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
       bash coreutils python3 tar \
    && rm -rf /var/lib/apt/lists/*

ENTRYPOINT ["/bin/bash"]
