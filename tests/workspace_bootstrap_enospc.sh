#!/bin/sh
set -eu

install -d /etc/ssh/platform /etc/workspace-platform /workspace/.mwc
ssh-keygen -q -t ed25519 -N '' -f /etc/ssh/platform/ssh_host_ed25519_key
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
test -e /run/mwc-ssh/home-degraded
test -e /run/mwc-ssh/reserve-released
test ! -e /workspace/.mwc/storage-reserve
