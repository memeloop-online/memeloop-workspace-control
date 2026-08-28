FROM tsl0922/ttyd@sha256:9a058c087bca6df456cb728514eafe778ea384369a622b3604c8b5256b67237d

USER root

RUN apt-get update \
    && apt-get install -y --no-install-recommends openssh-client \
    && rm -rf /var/lib/apt/lists/*

