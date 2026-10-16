#!/bin/sh
set -eu

install -d /etc/ssh/platform /etc/workspace-platform /workspace/.mwc
install -d -m 0750 -o workspace -g workspace /workspace/.codex /workspace/.codex/sessions /workspace/.codex/logs
install -d -m 0750 -o root -g root /workspace/.codex/.tmp
install -d -m 0750 -o root -g root /workspace/.codex/tmp/arg0/stale-root-owned
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
for failure in malformed non_array invalid_mode missing_key; do
    case "$failure" in
        malformed) printf '{' > "$injection_config/workspace-files.json" ;;
        non_array) printf '{"files":null}' > "$injection_config/workspace-files.json" ;;
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
