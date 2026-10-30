#!/usr/bin/env bash
set -euo pipefail

image=${1:?usage: workspace_cold_start.sh IMAGE FIXTURE_DIR BUILDKIT_IMAGE}
fixtures=${2:?missing generated fixture directory}
buildkit_image=${3:?missing pinned BuildKit image}
fixture_root=$(mktemp -d)
containers=()
scratch_mounts=()
success=false
cleanup() {
  for container in "${containers[@]}"; do
    if [ "$success" != true ]; then
      docker logs "$container" 2>&1 || true
    fi
    docker rm --force "$container" >/dev/null 2>&1 || true
  done
  for scratch_mount in "${scratch_mounts[@]}"; do
    sudo umount "$scratch_mount"
  done
  sudo rm -rf -- "$fixture_root"
}
trap cleanup EXIT

for enabled in false true; do
  fixture="$fixtures/buildkit-$enabled.json"
  root="$fixture_root/$enabled"
  workspace_container="mwc-cold-start-$$-$enabled"
  buildkit_container="mwc-cold-buildkit-$$-$enabled"
  mkdir -p "$root/home" "$root/scratch" "$root/config" "$root/identity" \
    "$root/injections/config" "$root/injections/secret"
  sudo mount -t tmpfs -o size=128m,mode=0755 tmpfs "$root/scratch"
  scratch_mounts+=("$root/scratch")
  # Kubelet creates the data subPath without exposing the scratch volume root.
  sudo mkdir -p "$root/scratch/data"
  jq -er '.config["mwc-workspace-bootstrap"]' "$fixture" > "$root/config/mwc-workspace-bootstrap"
  jq -er '.config.sshd_config' "$fixture" > "$root/config/sshd_config"
  chmod 0555 "$root/config/mwc-workspace-bootstrap"
  printf '%s' 'config value with spaces' > "$root/injections/config/session-config"
  printf '%s' 'secret-value' > "$root/injections/secret/session-secret"
  cat > "$root/injections/config/workspace-environment.json" <<'EOF'
{"environment":[
  {"name":"MWC_COLD_CONFIG","object":"config_map","data_key":"session-config"},
  {"name":"MWC_COLD_SECRET","object":"secret","data_key":"session-secret"}
]}
EOF
  if [ "$enabled" = true ]; then
    mapfile -t scratch_env < <(jq -r '.pod.initContainers[] | select(.name == "workspace-scratch-init") | .env[] | "-e", (.name + "=" + .value)' "$fixture")
    scratch_script=$(jq -er '.pod.initContainers[] | select(.name == "workspace-scratch-init") | .args[0]' "$fixture")
    docker run --rm --network none --entrypoint sh \
      "${scratch_env[@]}" \
      -v "$root/scratch/data:/var/lib/mwc/workspace-scratch" \
      "$image" -c "$scratch_script"
  else
    # Kubelet creates missing subPath directories before starting the root workspace container.
    # Leave them empty and root-owned; serve must prepare ownership and children itself.
    docker run --rm --network none --entrypoint sh \
      -v "$root/scratch/data:/var/lib/mwc/workspace-scratch" \
      "$image" -ec '
        install -d -m 0755 -o root -g root \
          /var/lib/mwc/workspace-scratch/workspace-cache \
          /var/lib/mwc/workspace-scratch/codex-session-scratch
      '
  fi
  # Kubelet creates a missing subPath before launching the main container.
  sudo mkdir -p "$root/scratch/data/tmp"

  # Seed durable data, not layout. Both cases enter serve without prepare-layout or prepare.
  docker run --rm --network none --entrypoint sh \
    -v "$root/home:/workspace" -v "$root/identity:/etc/ssh/platform" \
    "$image" -ec '
      install -d -m 0700 -o workspace -g workspace /workspace/.codex/sessions /workspace/.cache /workspace/.cargo
      printf "%s\n" keep-session > /workspace/.codex/sessions/keep
      printf "%s\n" keep-sqlite > /workspace/.codex/state.sqlite
      printf "%s\n" keep-cache > /workspace/.cache/keep
      printf "%s\n" keep-cargo-config > /workspace/.cargo/config.toml
      printf "%s\n" keep-npm-config > /workspace/.npmrc
      chown workspace:workspace /workspace/.codex/sessions/keep /workspace/.codex/state.sqlite /workspace/.cache/keep
      chown workspace:workspace /workspace/.cargo/config.toml /workspace/.npmrc
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
      -v "$root/scratch/data/build-cache:/var/lib/mwc-buildkit" \
      -v "$root/scratch/data/build-cache:/home/user/.local/share/buildkit" \
      "$buildkit_image" -ec 'test -z "$(find /var/lib/mwc-buildkit -mindepth 1 -print -quit)"'
    docker run --rm --network none --read-only --user 1000:1000 --cap-drop ALL \
      --security-opt no-new-privileges --entrypoint sh \
      -v "$root/scratch/data/build-cache:/var/lib/mwc-buildkit" \
      -v "$root/scratch/data/build-cache:/home/user/.local/share/buildkit" \
      "$buildkit_image" -c "$setup"
    containers+=("$buildkit_container")
    docker run --detach --name "$buildkit_container" --network none --privileged \
      -e TMPDIR=/var/lib/mwc-buildkit/tmp -e XDG_RUNTIME_DIR=/var/lib/mwc-buildkit/runtime \
      -e BUILDKIT_HOST=tcp://127.0.0.1:1234 \
      -v "$root/scratch/data/build-cache:/var/lib/mwc-buildkit" \
      -v "$root/scratch/data/build-cache:/home/user/.local/share/buildkit" \
      -v "$root/scratch/data/build-cache/tmp:/tmp" \
      "$buildkit_image" --config /var/lib/mwc-buildkit/config/buildkitd.toml >/dev/null
    network=(--network "container:$buildkit_container")
    buildkit_mount=(-v "$root/scratch/data/build-cache:/run/mwc-buildkit")
  fi

  mapfile -t workspace_env < <(jq -r '.pod.containers[] | select(.name == "workspace") | .env[] | select(has("value")) | "-e", (.name + "=" + .value)' "$fixture")
  # Model both image-provided Rustup locations; the controller must not override either one.
  rustup_home=/usr/local/rustup
  if [ "$enabled" = true ]; then rustup_home=/workspace/.rustup; fi
  containers+=("$workspace_container")
  docker run --detach --name "$workspace_container" "${network[@]}" \
    "${workspace_env[@]}" -e MWC_HOME_RESERVE_MIB=0 -e RUSTUP_HOME="$rustup_home" \
    --tmpfs /run/mwc-ssh:mode=0700 \
    -v "$root/scratch/data/tmp:/tmp" -v "$root/scratch/data/tmp:/var/tmp" \
    -v "$root/home:/workspace" -v "$root/config:/etc/workspace-platform:ro" \
    -v "$root/identity:/etc/ssh/platform:ro" \
    -v "$root/injections/config:/var/run/workspace-injections/config:ro" \
    -v "$root/injections/secret:/var/run/workspace-injections/secret:ro" \
    -v "$root/scratch/data/workspace-cache:/var/lib/mwc/build-scratch" \
    -v "$root/scratch/data/codex-session-scratch:/var/lib/mwc/codex-scratch" \
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
    test "$(cat /workspace/.cargo/config.toml)" = keep-cargo-config
    test "$(cat /workspace/.npmrc)" = keep-npm-config
    test "$(stat -c %a /workspace/.codex/state.sqlite)" = 600
    test "$(readlink /workspace/.codex/tmp)" = /var/lib/mwc/codex-scratch/tmp
    test "$(readlink /workspace/.codex/.tmp)" = /var/lib/mwc/codex-scratch/dot-tmp
    test "$(stat -c %U:%G /var/lib/mwc/build-scratch)" = workspace:workspace
    test "$(stat -c %U:%G /var/lib/mwc/codex-scratch)" = workspace:workspace
    test "$(stat -c %a /tmp)" = 1777
    test "$(stat -c %a /var/tmp)" = 1777
    test "$(stat -c %d /tmp)" = "$(stat -c %d /var/lib/mwc/build-scratch)"
    test "$(stat -c %d /var/tmp)" = "$(stat -c %d /tmp)"
    test "$(stat -c %d /workspace)" != "$(stat -c %d /tmp)"
  '
  # The container and non-login SSH sessions must use the same writable tool locations,
  # independently of a development image's inherited package-manager environment.
  # shellcheck disable=SC2016 # Expand these variables inside each tested session.
  tool_environment='test "$CARGO_HOME" = "$HOME/.cargo" &&
    test "$NPM_CONFIG_CACHE" = /var/lib/mwc/build-scratch/cache/npm &&
    test "$(readlink -f "$CARGO_HOME/registry")" = /var/lib/mwc/build-scratch/cargo-registry &&
    test "$(readlink -f "$CARGO_HOME/git")" = /var/lib/mwc/build-scratch/cargo-git &&
    mkdir -p "$NPM_CONFIG_CACHE" &&
    touch "$CARGO_HOME/registry/write-check" "$NPM_CONFIG_CACHE/write-check" \
      "$HOME/.codex/tmp/write-check" "$HOME/.codex/.tmp/write-check"'
  docker exec --user 1000:1000 "$workspace_container" sh -ec "$tool_environment"
  docker exec "$workspace_container" ssh -p 2222 \
    -o BatchMode=yes -o IdentitiesOnly=yes -o ConnectTimeout=1 -o StrictHostKeyChecking=yes \
    -o UserKnownHostsFile=/run/mwc-ssh/known_hosts \
    -i /run/mwc-ssh/ttyd_client_key workspace@127.0.0.1 "$tool_environment"
  test "$(docker exec --user 1000:1000 "$workspace_container" printenv RUSTUP_HOME)" = "$rustup_home"
  printf -v expected_environment '%s\n%s\n%s' "$rustup_home" 'config value with spaces' 'secret-value'
  # shellcheck disable=SC2016 # Expand the image environment in the non-login SSH session.
  test "$(docker exec "$workspace_container" ssh -p 2222 \
    -o BatchMode=yes -o IdentitiesOnly=yes -o ConnectTimeout=1 -o StrictHostKeyChecking=yes \
    -o UserKnownHostsFile=/run/mwc-ssh/known_hosts \
    -i /run/mwc-ssh/ttyd_client_key workspace@127.0.0.1 \
    'printf "%s\n" "$RUSTUP_HOME" "$MWC_COLD_CONFIG" "$MWC_COLD_SECRET"')" = "$expected_environment"
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
  # A full temporary filesystem rejects ordinary writes without consuming Home or stopping SSH.
  if docker exec --user 1000:1000 "$workspace_container" \
    env LC_ALL=C dd if=/dev/zero of=/tmp/capacity-fill bs=1M count=256 \
    2>"$root/tmp-full.stderr"; then
    echo "temporary storage unexpectedly exceeded its filesystem capacity" >&2
    exit 1
  fi
  grep -q 'No space left on device' "$root/tmp-full.stderr"
  docker exec "$workspace_container" ssh -p 2222 \
    -o BatchMode=yes -o IdentitiesOnly=yes -o ConnectTimeout=1 -o StrictHostKeyChecking=yes \
    -o UserKnownHostsFile=/run/mwc-ssh/known_hosts \
    -i /run/mwc-ssh/ttyd_client_key workspace@127.0.0.1 \
    'printf "%s\n" home-still-writable > "$HOME/tmp-capacity-home"'
  docker exec --user 1000:1000 "$workspace_container" sh -ec '
    ! printf x > /var/lib/mwc/build-scratch/cache/tmp-capacity-full
    rm /var/tmp/capacity-fill
    printf "%s\n" recovered > /tmp/tmp-capacity-recovered
    test "$(cat /var/tmp/tmp-capacity-recovered)" = recovered
    test "$(cat /workspace/tmp-capacity-home)" = home-still-writable
  '
  echo "cold serve strict SSH and durable-data preservation passed (buildkit=$enabled)"
done
success=true
