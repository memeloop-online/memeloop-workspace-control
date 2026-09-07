#!/bin/sh
set -eu

install -d /etc/ssh/platform /etc/workspace-platform /workspace/.mwc
install -d -m 0750 -o workspace -g workspace /workspace/.codex /workspace/.codex/sessions
install -d -m 0750 -o root -g root /workspace/.codex/.tmp
printf '%s\n' stale > /workspace/.codex/.tmp/stale
printf '%s\n' keep > /workspace/.codex/sessions/keep
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
test "$(readlink /workspace/.codex/.tmp)" = /var/lib/mwc/codex-scratch
test ! -e /var/lib/mwc/codex-scratch/stale
test -s /workspace/.codex/sessions/keep

# A stale symlink or regular file must be removed without following it, while the durable
# session directory remains untouched.
rm -f /workspace/.codex/.tmp
ln -s /workspace/.codex/sessions /workspace/.codex/.tmp
MWC_WORKSPACE_USER=workspace \
MWC_WORKSPACE_HOME=/workspace \
MWC_HOME_RESERVE_MIB=1 \
  /usr/local/bin/mwc-workspace-bootstrap prepare
test -L /workspace/.codex/.tmp
test "$(readlink /workspace/.codex/.tmp)" = /var/lib/mwc/codex-scratch
test -s /workspace/.codex/sessions/keep

rm -f /workspace/.codex/.tmp
printf '%s\n' stale-file > /workspace/.codex/.tmp
MWC_WORKSPACE_USER=workspace \
MWC_WORKSPACE_HOME=/workspace \
MWC_HOME_RESERVE_MIB=1 \
  /usr/local/bin/mwc-workspace-bootstrap prepare
test -L /workspace/.codex/.tmp
test "$(readlink /workspace/.codex/.tmp)" = /var/lib/mwc/codex-scratch
