#!/bin/bash
# Runs only on the disposable GitHub-hosted acceptance VM.
set -euo pipefail
: "${GITHUB_ACTIONS:?GitHub Actions only}"
: "${GITHUB_WORKSPACE:?}" "${RUNNER_TEMP:?}" "${PRODUCT_RENDERER_SHA:?}"
: "${MWC_PRODUCT_STORAGE_WORKSPACE_IMAGE:?}"
inputs="${1:?Input directory required}"
mkdir -p "$inputs/registry" "$GITHUB_WORKSPACE/artifacts"
exec > >(tee "$GITHUB_WORKSPACE/artifacts/public-input-prepare.log") 2>&1
fetch() {
 local name="$1" url="$2" digest="$3"
 curl --fail --location --silent --show-error --retry 3 --connect-timeout 15 --max-time 600 "$url" -o "$inputs/$name"
 printf '%s  %s\n' "$digest" "$inputs/$name" | sha256sum -c -
}
fetch k3s 'https://github.com/k3s-io/k3s/releases/download/v1.36.2%2Bk3s1/k3s' 65a55ec56c24eab44383086166ec620a491952b7e23941a49ddca6e8a4c4b4de
fetch gvisor.tar.bz2 'https://github.com/gvisor/gvisor/releases/download/release-20261019.0/gvisor-x86_64.tar.bz2' 1298e8ed9c3fee900f00d2ea96ec2d79d80ed93f2175812f29b7e7da718041a3
fetch helm.tgz 'https://get.helm.sh/helm-v4.2.4-linux-amd64.tar.gz' c306b46f719b0a4da32d0f78ee21bf90ce8d602f15b22ab753f0674d1670a7f3
fetch topolvm.tgz 'https://github.com/topolvm/topolvm/releases/download/topolvm-chart-v17.2.0/topolvm-17.2.0.tgz' 4b6c070da0b38f098c9ef6b6a38ef4e2dfff52adecedc280ad92bfab408f90ee
# rust-dev is an independently maintained private Forgejo image, not a GHCR
# cache. This new acceptance image makes no claim to validate that old digest.
context="$RUNNER_TEMP/product-storage-image"
mkdir -p "$context"
cat > "$context/Dockerfile" <<'DOCKERFILE'
ARG BASE_IMAGE
FROM ${BASE_IMAGE}
USER root
# The renderer mounts /home/user from a fresh PVC and sets CARGO_HOME there.
# Keep rustup and its proxy binaries in the image, outside that mounted Home.
ENV RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
 ca-certificates jq openssh-client openssh-server sudo tini \
 && groupadd --gid 1000 user \
 && useradd --uid 1000 --gid 1000 --create-home --shell /bin/bash user \
 && passwd --delete user \
 && mkdir -p /run/sshd && rm -rf /var/lib/apt/lists/*
# Verify the renderer/sshd environment as UID 1000 with an empty Home. No
# toolchain seed in /home/user may be required: that directory is a PVC later.
RUN test "$(id -u user)" = 1000 && test "$(id -g user)" = 1000 \
 && test -x /usr/sbin/sshd && command -v jq && command -v runuser \
 && install -d -m 0700 -o 1000 -g 1000 /tmp/mwc-empty-home \
 && runuser --user user -- env HOME=/tmp/mwc-empty-home \
    CARGO_HOME=/tmp/mwc-empty-home/.cargo RUSTUP_HOME=/usr/local/rustup \
    PATH=/usr/local/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
    sh -ec 'rustc --version; cargo --version; rustup which rustc' \
 && rm -rf /tmp/mwc-empty-home
DOCKERFILE
docker pull --platform linux/amd64 docker.io/library/rust:1.98-bookworm
base="$(docker image inspect docker.io/library/rust:1.98-bookworm --format '{{index .RepoDigests 0}}')"
[[ "$base" =~ @sha256:[a-f0-9]{64}$ ]]
docker build --platform linux/amd64 --build-arg "BASE_IMAGE=$base" \
 --label "org.opencontainers.image.revision=$PRODUCT_RENDERER_SHA" \
 --tag "$MWC_PRODUCT_STORAGE_WORKSPACE_IMAGE" "$context"
image_id="$(docker image inspect "$MWC_PRODUCT_STORAGE_WORKSPACE_IMAGE" --format '{{.Id}}')"
docker save --output "$inputs/workspace-image.tar" "$MWC_PRODUCT_STORAGE_WORKSPACE_IMAGE"
# The archive is now the sole handoff to the test's actual containerd. Release
# this job's duplicate Docker copy; never prune unrelated runner resources.
docker image rm "$MWC_PRODUCT_STORAGE_WORKSPACE_IMAGE" docker.io/library/rust:1.98-bookworm
jq -n --arg source "$PRODUCT_RENDERER_SHA" --arg image "$MWC_PRODUCT_STORAGE_WORKSPACE_IMAGE" \
 --arg config "$image_id" --arg base "$base" --arg run "$GITHUB_RUN_ID" --arg attempt "$GITHUB_RUN_ATTEMPT" \
 --arg dockerfile_sha "$(sha256sum "$context/Dockerfile" | cut -d' ' -f1)" \
 '{mode:"github-public",source_sha:$source,run_id:$run,run_attempt:$attempt,workspace_image:$image,workspace_config_digest:$config,workspace_uid:1000,workspace_gid:1000,rustup_home:"/usr/local/rustup",cargo_home_at_runtime:"/home/user/.cargo",home_toolchain_seed_required:false,public_base:$base,dockerfile_sha256:$dockerfile_sha,production_rust_d507_tested:false,harbor_required:false}' \
 > "$inputs/registry/public-input-identity.json"
cp "$inputs/registry/public-input-identity.json" "$GITHUB_WORKSPACE/artifacts/"
(cd "$inputs" && sha256sum k3s gvisor.tar.bz2 helm.tgz topolvm.tgz workspace-image.tar registry/*.json > SHA256SUMS)
