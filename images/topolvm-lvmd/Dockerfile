ARG BASE_IMAGE=ghcr.io/memeloop-online/memeloop-workspace-control-topolvm-controller@sha256:4d3edf502ca487d75c02bf9f5e55bdb4e1d65eb966987b46a782cf4ed23c360f
FROM ${BASE_IMAGE}

RUN --mount=type=bind,source=lvmd,target=/tmp/lvmd-patched \
    rm /lvmd && install -m 0755 /tmp/lvmd-patched /lvmd
