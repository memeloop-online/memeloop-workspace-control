#!/usr/bin/env bash
set -euo pipefail

image=${1:?usage: workspace_cold_start.sh IMAGE FIXTURE_DIR BUILDKIT_IMAGE}
fixtures=${2:?missing generated fixture directory}
buildkit_image=${3:?missing pinned BuildKit image}
fixture_root=$(mktemp -d)
containers=()
success=false
cleanup() {
  for container in "${containers[@]}"; do
    if [ "$success" != true ]; then
      docker logs "$container" 2>&1 || true
    fi
    docker rm --force "$container" >/dev/null 2>&1 || true
  done
  sudo rm -rf -- "$fixture_root"
}
trap cleanup EXIT

for enabled in false true; do
  fixture="$fixtures/buildkit-$enabled.json"
  root="$fixture_root/$enabled"
  workspace_container="mwc-cold-start-$$-$enabled"
  buildkit_container="mwc-cold-buildkit-$$-$enabled"
  mkdir -p "$root/home" "$root/scratch" "$root/config" "$root/identity"
  jq -er '.config["mwc-workspace-bootstrap"]' "$fixture" > "$root/config/mwc-workspace-bootstrap"
  jq -er '.config.sshd_config' "$fixture" > "$root/config/sshd_config"
  chmod 0555 "$root/config/mwc-workspace-bootstrap"
  mapfile -t scratch_env < <(jq -r '.pod.initContainers[] | select(.name == "workspace-scratch-init") | .env[] | "-e", (.name + "=" + .value)' "$fixture")
  scratch_script=$(jq -er '.pod.initContainers[] | select(.name == "workspace-scratch-init") | .args[0]' "$fixture")
  docker run --rm --network none --entrypoint sh \
    "${scratch_env[@]}" \
    -v "$root/scratch:/var/lib/mwc/workspace-scratch" \
    "$image" -c "$scratch_script"

  # Seed durable data, not layout. Both cases enter serve without prepare-layout or prepare.
  docker run --rm --network none --entrypoint sh \
    -v "$root/home:/workspace" -v "$root/identity:/etc/ssh/platform" \
    "$image" -ec '
      install -d -m 0700 -o workspace -g workspace /workspace/.codex/sessions /workspace/.cache
      printf "%s\n" keep-session > /workspace/.codex/sessions/keep
      printf "%s\n" keep-sqlite > /workspace/.codex/state.sqlite
      printf "%s\n" keep-cache > /workspace/.cache/keep
      chown workspace:workspace /workspace/.codex/sessions/keep /workspace/.codex/state.sqlite /workspace/.cache/keep
      chmod 0600 /workspace/.codex/sessions/keep /workspace/.codex/state.sqlite /workspace/.cache/keep
      chown root:root /workspace
      chmod 0777 /workspace
      ssh-keygen -q -t ed25519 -N "" -f /etc/ssh/platform/ssh_host_ed25519_key
    '

  network=(--network none)
  buildkit_mount=()
  if [ "$enabled" = true ]; then
    setup=$(jq -er '.pod.initContainers[] | select(.name == "buildkit-bootstrap") | .args[0]' "$fixture")
    # Only scratch-init has run: the UID1000 bootstrap must create every child itself.
    docker run --rm --network none --read-only --user 1000:1000 --cap-drop ALL \
      --security-opt no-new-privileges --entrypoint sh \
      -v "$root/scratch/build-cache:/var/lib/mwc-buildkit" \
      "$buildkit_image" -ec 'test -z "$(find /var/lib/mwc-buildkit -mindepth 1 -print -quit)"'
    docker run --rm --network none --read-only --user 1000:1000 --cap-drop ALL \
      --security-opt no-new-privileges --entrypoint sh \
      -v "$root/scratch/build-cache:/var/lib/mwc-buildkit" \
      "$buildkit_image" -c "$setup"
    containers+=("$buildkit_container")
    docker run --detach --name "$buildkit_container" --network none --privileged \
      -e TMPDIR=/var/lib/mwc-buildkit/tmp -e XDG_RUNTIME_DIR=/var/lib/mwc-buildkit/runtime \
      -e BUILDKIT_HOST=tcp://127.0.0.1:1234 \
      -v "$root/scratch/build-cache:/var/lib/mwc-buildkit" \
      -v "$root/scratch/build-cache/tmp:/tmp" \
      "$buildkit_image" --config /var/lib/mwc-buildkit/config/buildkitd.toml >/dev/null
    network=(--network "container:$buildkit_container")
    buildkit_mount=(-v "$root/scratch/build-cache:/run/mwc-buildkit")
  fi

  mapfile -t workspace_env < <(jq -r '.pod.containers[] | select(.name == "workspace") | .env[] | select(has("value")) | "-e", (.name + "=" + .value)' "$fixture")
  containers+=("$workspace_container")
  docker run --detach --name "$workspace_container" "${network[@]}" \
    "${workspace_env[@]}" -e MWC_HOME_RESERVE_MIB=0 \
    --tmpfs /run/mwc-ssh:mode=0700 --tmpfs /tmp:mode=1777 --tmpfs /var/tmp:mode=1777 \
    -v "$root/home:/workspace" -v "$root/config:/etc/workspace-platform:ro" \
    -v "$root/identity:/etc/ssh/platform:ro" \
    -v "$root/scratch/workspace-cache:/var/lib/mwc/build-scratch" \
    -v "$root/scratch/codex-session-scratch:/var/lib/mwc/codex-scratch" \
    "${buildkit_mount[@]}" --entrypoint /etc/workspace-platform/mwc-workspace-bootstrap \
    "$image" serve >/dev/null

  ready=false
  for _attempt in $(seq 1 30); do
    if docker exec "$workspace_container" ssh -p 2222 \
      -o BatchMode=yes -o IdentitiesOnly=yes -o ConnectTimeout=1 -o StrictHostKeyChecking=yes \
      -o UserKnownHostsFile=/run/mwc-ssh/known_hosts \
      -i /run/mwc-ssh/ttyd_client_key workspace@127.0.0.1 'test "$(id -u)" = 1000'; then
      ready=true
      break
    fi
    sleep 1
  done
  test "$ready" = true
  docker exec "$workspace_container" sh -ec '
    test "$(stat -c %U:%G /workspace)" = workspace:workspace
    test "$(stat -c %a /workspace)" = 750
    test "$(stat -c %a /run/mwc-ssh)" = 710
    test "$(stat -c %a /run/mwc-ssh/ssh_host_ed25519_key)" = 600
    test "$(cat /workspace/.codex/sessions/keep)" = keep-session
    test "$(cat /workspace/.codex/state.sqlite)" = keep-sqlite
    test "$(cat /workspace/.cache/keep)" = keep-cache
    test "$(stat -c %a /workspace/.codex/state.sqlite)" = 600
    test "$(readlink /workspace/.codex/tmp)" = /var/lib/mwc/codex-scratch/tmp
    test "$(readlink /workspace/.codex/.tmp)" = /var/lib/mwc/codex-scratch/dot-tmp
  '
  if [ "$enabled" = true ]; then
    ready=false
    for _attempt in $(seq 1 30); do
      if docker exec --user 1000:1000 "$workspace_container" \
        /run/mwc-buildkit/bin/buildctl --addr tcp://127.0.0.1:1234 debug workers | grep -q linux; then
        ready=true
        break
      fi
      sleep 1
    done
    test "$ready" = true
    docker exec "$buildkit_container" sh -ec 'test "$(id -u)" = 1000'
  fi
  echo "cold serve strict SSH and durable-data preservation passed (buildkit=$enabled)"
done
success=true
