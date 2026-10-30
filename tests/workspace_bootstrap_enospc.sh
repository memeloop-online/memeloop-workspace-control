#!/bin/sh
set -eu
unset RUSTUP_HOME

install -d /etc/ssh/platform /etc/workspace-platform /workspace/.mwc
install -d -m 0750 -o workspace -g workspace /workspace/.codex /workspace/.codex/sessions /workspace/.codex/logs
install -d -m 0750 -o root -g root /workspace/.codex/.tmp
install -d -m 0750 -o root -g root /workspace/.codex/tmp/arg0/stale-root-owned
install -d -m 0755 -o root -g root /workspace/.codex/skills/user-skill
printf '%s\n' keep > /workspace/.codex/skills/user-skill/SKILL.md
printf '%s\n' stale > /workspace/.codex/.tmp/stale
printf '%s\n' stale > /workspace/.codex/tmp/arg0/stale-root-owned/lock
printf '%s\n' keep > /workspace/.codex/sessions/keep
printf '%s\n' keep > /workspace/.codex/logs/keep
printf '%s\n' keep > /workspace/.codex/logs_2.sqlite
printf '%s\n' keep > /workspace/.codex/logs_2.sqlite-shm
printf '%s\n' keep > /workspace/.codex/logs_2.sqlite-wal
printf '%s\n' keep > /workspace/.codex/session_index.jsonl
printf '%s\n' keep > /workspace/.codex/config.toml
printf '%s\n' keep > /workspace/.codex/auth.json
ssh-keygen -q -t ed25519 -N '' -f /etc/ssh/platform/ssh_host_ed25519_key
# Exercise the common Debian service-account state that previously rejected public keys before
# AuthorizedKeysFile was consulted.
usermod -p '!locked' workspace
cat > /etc/workspace-platform/sshd_config <<'EOF'
Port 2222
HostKey /run/mwc-ssh/ssh_host_ed25519_key
AuthorizedKeysFile /run/mwc-ssh/authorized_keys
PidFile /run/mwc-ssh/sshd.pid
PasswordAuthentication no
EOF

# The reserve is the only Home object that the platform may remove under critical pressure.
fallocate -l 1M /workspace/.mwc/storage-reserve
dd if=/dev/zero of=/workspace/fill bs=1M count=64 2>/dev/null || true
test "$(df -P /workspace | awk 'NR == 2 {print $5}')" = "100%"

MWC_WORKSPACE_USER=workspace \
MWC_WORKSPACE_HOME=/workspace \
MWC_HOME_RESERVE_MIB=1 \
  /usr/local/bin/mwc-workspace-bootstrap prepare

test -s /run/mwc-ssh/authorized_keys
test -s /run/mwc-ssh/ttyd_client_key
test -s /run/mwc-ssh/ssh_host_ed25519_key
test -s /run/mwc-ssh/known_hosts
test -s /run/mwc-ssh/sshd_config
/usr/sbin/sshd -t -f /run/mwc-ssh/sshd_config
test "$(getent shadow workspace | cut -d: -f2)" = "NP"
test "$(stat -c %a /run/mwc-ssh)" = "710"
test "$(stat -c %U:%G /run/mwc-ssh)" = "root:workspace"
su -s /bin/sh workspace -c 'test -r /run/mwc-ssh/authorized_keys'
test "$(stat -c %a /workspace)" = "750"
test "$(stat -c %U:%G /workspace)" = "workspace:workspace"
test -e /run/mwc-ssh/reserve-released
test ! -e /workspace/.mwc/storage-reserve
test "$(df -P /workspace | awk 'NR == 2 {print $5}')" != "100%"
test -s /run/mwc-ssh/storage-banner
test -L /workspace/.codex/.tmp
test "$(readlink /workspace/.codex/.tmp)" = /var/lib/mwc/codex-scratch/dot-tmp
test -L /workspace/.codex/tmp
test "$(readlink /workspace/.codex/tmp)" = /var/lib/mwc/codex-scratch/tmp
test ! -e /var/lib/mwc/codex-scratch/dot-tmp/stale
test ! -e /var/lib/mwc/codex-scratch/tmp/arg0
test -s /workspace/.codex/sessions/keep
test -s /workspace/.codex/logs/keep
test -s /workspace/.codex/logs_2.sqlite
test -s /workspace/.codex/logs_2.sqlite-shm
test -s /workspace/.codex/logs_2.sqlite-wal
test -s /workspace/.codex/session_index.jsonl
test -s /workspace/.codex/config.toml
test -s /workspace/.codex/auth.json
test "$(stat -c %U:%G /workspace/.codex/skills)" = workspace:workspace
su -s /bin/sh workspace -c 'mkdir /workspace/.codex/skills/.system; printf installed > /workspace/.codex/skills/.system/marker'
test "$(cat /workspace/.codex/skills/user-skill/SKILL.md)" = keep
test "$(stat -c %U:%G /workspace/.codex/skills/user-skill/SKILL.md)" = root:root

# A failed injection must keep SSH keys available and recover on the next complete sync,
# without requiring a workspace restart. All data below belongs to this isolated fixture.
rm -f /workspace/fill
install -d /var/run/workspace-injections/config
injection_config=/var/run/workspace-injections/config
printf '%s\n' old > "$injection_config/recovery-file"
cp /etc/ssh/platform/ssh_host_ed25519_key.pub "$injection_config/recovery-key"
cat > "$injection_config/workspace-files.json" <<'EOF'
{"files":[
  {"object":"config_map","data_key":"recovery-file","target":"/workspace/recovery-file","kind":"file","mode":416},
  {"object":"config_map","data_key":"recovery-key","target":"/workspace/.ssh/authorized_keys","kind":"ssh_public_key"}
]}
EOF
prepare_recovery_fixture() {
    MWC_WORKSPACE_USER=workspace MWC_WORKSPACE_HOME=/workspace MWC_HOME_RESERVE_MIB=1 \
        /usr/local/bin/mwc-workspace-bootstrap prepare
    /usr/sbin/sshd -t -f /run/mwc-ssh/sshd_config
    test -s /run/mwc-ssh/authorized_keys
    test -s /run/mwc-ssh/ttyd_client_key
}
prepare_recovery_fixture
test ! -e /run/mwc-ssh/injection-failed
test "$(cat /workspace/recovery-file)" = old
test "$(stat -c %a /workspace/recovery-file)" = 640
test "$(stat -c %U:%G /workspace/recovery-file)" = workspace:workspace
cp /run/mwc-ssh/authorized_keys /tmp/expected-authorized-keys

# A regular file in place of a parent directory forces a persistent injection failure.
printf '%s\n' keep > /workspace/blocked-parent
printf '%s\n' new > "$injection_config/recovery-file"
jq '.files += [{"object":"config_map","data_key":"recovery-file","target":"/workspace/blocked-parent/config","kind":"file"}]' \
    "$injection_config/workspace-files.json" > /tmp/recovery-manifest
cp /tmp/recovery-manifest "$injection_config/workspace-files.json"
prepare_recovery_fixture
test -e /run/mwc-ssh/injection-failed
grep -q '文件注入未能完全更新' /run/mwc-ssh/storage-banner
test "$(cat /workspace/blocked-parent)" = keep
cmp /tmp/expected-authorized-keys /run/mwc-ssh/authorized_keys

rm /workspace/blocked-parent
mkdir /workspace/blocked-parent
prepare_recovery_fixture
test ! -e /run/mwc-ssh/injection-failed
test "$(cat /workspace/recovery-file)" = new
test "$(cat /workspace/blocked-parent/config)" = new
! grep -q '文件注入未能完全更新' /run/mwc-ssh/storage-banner
cmp /tmp/expected-authorized-keys /run/mwc-ssh/authorized_keys

# Parse failures, invalid modes and missing SSH sources must never clear the warning or
# replace working authorized keys with a partial key set. Every case must recover in place.
cp "$injection_config/workspace-files.json" /tmp/good-injection-manifest
for failure in malformed non_array invalid_metadata invalid_mode missing_key; do
    case "$failure" in
        malformed) printf '{' > "$injection_config/workspace-files.json" ;;
        non_array) printf '{"files":null}' > "$injection_config/workspace-files.json" ;;
        invalid_metadata) jq '.files[0].owner = []' /tmp/good-injection-manifest > "$injection_config/workspace-files.json" ;;
        invalid_mode) jq '.files[0].mode = "invalid"' /tmp/good-injection-manifest > "$injection_config/workspace-files.json" ;;
        missing_key) jq '.files[1].data_key = "missing-key"' /tmp/good-injection-manifest > "$injection_config/workspace-files.json" ;;
    esac
    prepare_recovery_fixture
    test -e /run/mwc-ssh/injection-failed
    cmp /tmp/expected-authorized-keys /run/mwc-ssh/authorized_keys
    test "$(cat /workspace/recovery-file)" = new
    cp /tmp/good-injection-manifest "$injection_config/workspace-files.json"
    prepare_recovery_fixture
    test ! -e /run/mwc-ssh/injection-failed
    cmp /tmp/expected-authorized-keys /run/mwc-ssh/authorized_keys
done
test -z "$(find /run/mwc-ssh -maxdepth 1 -name '.injection-sync.*' -print -quit)"

# File metadata is data even when a target contains shell syntax or embedded newlines.
special_target=$(cat <<'EOF'
/workspace/metadata space 'single' "double" \backslash
line $(touch /tmp/mwc-files-command-substitution-ran) `touch /tmp/mwc-files-backtick-ran` end
EOF
)
test ! -e /tmp/mwc-files-command-substitution-ran
test ! -e /tmp/mwc-files-backtick-ran
printf '%s' 'exact metadata payload' > "$injection_config/metadata-file"
jq --arg target "$special_target" '.files += [{
    object:"config_map",data_key:"metadata-file",target:$target,kind:"config_file",
    mode:416,owner:"workspace",group:"workspace"
}]' /tmp/good-injection-manifest > "$injection_config/workspace-files.json"
prepare_recovery_fixture
test ! -e /run/mwc-ssh/injection-failed
cmp "$injection_config/metadata-file" "$special_target"
test "$(stat -c %a "$special_target")" = 640
test "$(stat -c %U:%G "$special_target")" = workspace:workspace
test ! -e /tmp/mwc-files-command-substitution-ran
test ! -e /tmp/mwc-files-backtick-ran
cmp /tmp/expected-authorized-keys /run/mwc-ssh/authorized_keys
cp /tmp/good-injection-manifest "$injection_config/workspace-files.json"

# Parse environment metadata together while reading each actual value from its original
# Secret/ConfigMap file. Reserved fields must be skipped before their sources are opened.
install -d /var/run/workspace-injections/secret
environment_secret=/var/run/workspace-injections/secret
config_value=$(cat <<'EOF'
配置 "双引号" \path with spaces
EOF
)
secret_value=$(cat <<'EOF'
秘密 '单引号' "double" \path $HOME
EOF
)
printf '%s' "$config_value" > "$injection_config/metadata-config"
printf '%s' "$secret_value" > "$environment_secret/metadata-secret"
cat > "$injection_config/workspace-environment.json" <<'EOF'
{"environment":[
  {"name":"MWC_METADATA_CONFIG","object":"config_map","data_key":"metadata-config"},
  {"name":"PATH","object":"config_map","data_key":"reserved-missing"},
  {"name":"MWC_METADATA_SECRET","object":"secret","data_key":"metadata-secret"},
  {"name":"CARGO_HOME","object":"config_map","data_key":"reserved-missing"},
  {"name":"RUSTUP_HOME","object":"secret","data_key":"reserved-missing"},
  {"name":"NPM_CONFIG_CACHE","object":"secret","data_key":"reserved-missing"},
  {"name":"HOME","object":"secret","data_key":"reserved-missing"}
]}
EOF
prepare_recovery_fixture
cat > /tmp/expected-metadata-setenv <<'EOF'
SetEnv "MWC_METADATA_CONFIG=配置 \"双引号\" \\path with spaces" "MWC_METADATA_SECRET=秘密 '单引号' \"double\" \\path $HOME"
EOF
while IFS= read -r expected_line; do
    grep -Fx "$expected_line" /run/mwc-ssh/sshd_config
done < /tmp/expected-metadata-setenv
test "$(grep -c '^SetEnv ' /run/mwc-ssh/sshd_config)" = 1
test -z "$(find /run/mwc-ssh -maxdepth 1 -name '.environment-metadata.*' -print -quit)"
cp "$injection_config/workspace-environment.json" /tmp/good-environment-manifest
for failure in malformed non_array missing_value invalid_object; do
    case "$failure" in
        malformed) printf '{' > "$injection_config/workspace-environment.json" ;;
        non_array) printf '{"environment":null}' > "$injection_config/workspace-environment.json" ;;
        missing_value) jq '.environment[0].data_key = "missing-value"' /tmp/good-environment-manifest > "$injection_config/workspace-environment.json" ;;
        invalid_object) jq '.environment[0].object = "unknown"' /tmp/good-environment-manifest > "$injection_config/workspace-environment.json" ;;
    esac
    if MWC_WORKSPACE_USER=workspace MWC_WORKSPACE_HOME=/workspace MWC_HOME_RESERVE_MIB=1 \
        /usr/local/bin/mwc-workspace-bootstrap prepare; then
        echo "invalid environment metadata unexpectedly succeeded: $failure" >&2
        exit 1
    fi
    test -z "$(find /run/mwc-ssh -maxdepth 1 -name '.environment-metadata.*' -print -quit)"
    cp /tmp/good-environment-manifest "$injection_config/workspace-environment.json"
    prepare_recovery_fixture
    test "$(grep -c '^SetEnv ' /run/mwc-ssh/sshd_config)" = 1
done
rm "$injection_config/workspace-environment.json"

# Image-provided Rustup paths use the same SSH quoting as injected values. Empty or absent
# image values must not create a platform default, and injected values cannot replace them.
RUSTUP_HOME='/workspace/toolchains "quoted" \path' prepare_recovery_fixture
grep -Fx 'SetEnv "RUSTUP_HOME=/workspace/toolchains \"quoted\" \\path"' /run/mwc-ssh/sshd_config
RUSTUP_HOME= prepare_recovery_fixture
! grep -q 'RUSTUP_HOME=' /run/mwc-ssh/sshd_config
unset RUSTUP_HOME
prepare_recovery_fixture
! grep -q 'RUSTUP_HOME=' /run/mwc-ssh/sshd_config

# Dictionary transport must restore exact bytes, including Unicode, CRLF, escaped newlines
# and the absence of a final newline. A failed decode must preserve the last good file.
printf '中文🦀 "quotes"\r\n literal\\nnext\n' > /tmp/dictionary-chunk
printf 'no trailing newline' > /tmp/dictionary-tail
cat /tmp/dictionary-chunk /tmp/dictionary-chunk /tmp/dictionary-tail > /tmp/dictionary-expected
jq -nc --rawfile chunk /tmp/dictionary-chunk --rawfile tail /tmp/dictionary-tail \
    '{version:1,chunks:[$chunk,$tail],order:[0,0,1]}' > /tmp/good-dictionary
jq --argjson bytes "$(wc -c < /tmp/dictionary-expected)" '.files += [{
    object:"config_map",data_key:"dictionary-file",target:"/workspace/dictionary-file",
    kind:"config_file",encoding:"dictionary_v1",decoded_bytes:$bytes,mode:416,
    owner:"workspace",group:"workspace"
}]' /tmp/good-injection-manifest > /tmp/dictionary-manifest
cp /tmp/dictionary-manifest "$injection_config/workspace-files.json"
cp /tmp/good-dictionary "$injection_config/dictionary-file"
prepare_recovery_fixture
test ! -e /run/mwc-ssh/injection-failed
cmp /tmp/dictionary-expected /workspace/dictionary-file
test "$(stat -c %a /workspace/dictionary-file)" = 640
test "$(stat -c %U:%G /workspace/dictionary-file)" = workspace:workspace
cmp /tmp/expected-authorized-keys /run/mwc-ssh/authorized_keys
for failure in bad_index fractional_index non_string wrong_version truncated size unknown_encoding encoded_ssh encoded_secret; do
    case "$failure" in
        bad_index) jq '.order = [0,99]' /tmp/good-dictionary > "$injection_config/dictionary-file" ;;
        fractional_index) jq '.order = [0,0.5]' /tmp/good-dictionary > "$injection_config/dictionary-file" ;;
        non_string) jq '.chunks[0] = null' /tmp/good-dictionary > "$injection_config/dictionary-file" ;;
        wrong_version) jq '.version = 2' /tmp/good-dictionary > "$injection_config/dictionary-file" ;;
        truncated) printf '{' > "$injection_config/dictionary-file" ;;
        size) jq '.files[-1].decoded_bytes += 1' /tmp/dictionary-manifest > "$injection_config/workspace-files.json" ;;
        unknown_encoding) jq '.files[-1].encoding = "unknown"' /tmp/dictionary-manifest > "$injection_config/workspace-files.json" ;;
        encoded_ssh) jq '.files[-1].kind = "ssh_public_key"' /tmp/dictionary-manifest > "$injection_config/workspace-files.json" ;;
        encoded_secret) jq '.files[-1].object = "secret"' /tmp/dictionary-manifest > "$injection_config/workspace-files.json" ;;
    esac
    prepare_recovery_fixture
    test -e /run/mwc-ssh/injection-failed
    cmp /tmp/dictionary-expected /workspace/dictionary-file
    cmp /tmp/expected-authorized-keys /run/mwc-ssh/authorized_keys
    cp /tmp/good-dictionary "$injection_config/dictionary-file"
    cp /tmp/dictionary-manifest "$injection_config/workspace-files.json"
    prepare_recovery_fixture
    test ! -e /run/mwc-ssh/injection-failed
    cmp /tmp/dictionary-expected /workspace/dictionary-file
    cmp /tmp/expected-authorized-keys /run/mwc-ssh/authorized_keys
done
test -z "$(find /run/mwc-ssh -maxdepth 1 -name '.injection-sync.*' -print -quit)"
test -z "$(find /workspace -name '.mwc-injection.*' -print -quit)"
cp /tmp/good-injection-manifest "$injection_config/workspace-files.json"

# A stale symlink or regular file must be removed without following it, while the durable
# session directory remains untouched.
rm -f /workspace/.codex/.tmp
ln -s /workspace/.codex/sessions /workspace/.codex/.tmp
MWC_WORKSPACE_USER=workspace \
MWC_WORKSPACE_HOME=/workspace \
MWC_HOME_RESERVE_MIB=1 \
  /usr/local/bin/mwc-workspace-bootstrap prepare
test -L /workspace/.codex/.tmp
test "$(readlink /workspace/.codex/.tmp)" = /var/lib/mwc/codex-scratch/dot-tmp
test -s /workspace/.codex/sessions/keep
test -s /workspace/.codex/logs/keep
test -s /workspace/.codex/logs_2.sqlite
test -s /workspace/.codex/logs_2.sqlite-shm
test -s /workspace/.codex/logs_2.sqlite-wal
test -s /workspace/.codex/session_index.jsonl
test -s /workspace/.codex/config.toml
test -s /workspace/.codex/auth.json

rm -f /workspace/.codex/.tmp
printf '%s\n' stale-file > /workspace/.codex/.tmp
MWC_WORKSPACE_USER=workspace \
MWC_WORKSPACE_HOME=/workspace \
MWC_HOME_RESERVE_MIB=1 \
  /usr/local/bin/mwc-workspace-bootstrap prepare
test -L /workspace/.codex/.tmp
test "$(readlink /workspace/.codex/.tmp)" = /var/lib/mwc/codex-scratch/dot-tmp
test -s /workspace/.codex/sessions/keep
test -s /workspace/.codex/logs/keep
test -s /workspace/.codex/logs_2.sqlite
test -s /workspace/.codex/logs_2.sqlite-shm
test -s /workspace/.codex/logs_2.sqlite-wal
test -s /workspace/.codex/session_index.jsonl
test -s /workspace/.codex/config.toml
test -s /workspace/.codex/auth.json
