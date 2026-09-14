#!/usr/bin/env bash
set -euo pipefail

image=${1:?usage: kali_browser_desktop_smoke.sh IMAGE}
container="mwc-kali-browser-desktop-smoke-$$"

cleanup() {
  if [ "${success:-false}" != true ]; then
    docker logs "$container" 2>&1 || true
  fi
  docker rm --force "$container" >/dev/null 2>&1 || true
}
trap cleanup EXIT

docker run --detach --name "$container" \
  -e MWC_DESKTOP_ENABLED=true \
  -e MWC_DESKTOP_PORT=6080 \
  --entrypoint /bin/sh \
  "$image" -ec '
    install -d -m 0700 /etc/ssh/platform /etc/workspace-platform
    ssh-keygen -q -t ed25519 -N "" -f /etc/ssh/platform/ssh_host_ed25519_key
    printf "%s\\n" \
      "Port 2222" \
      "ListenAddress 127.0.0.1" \
      "HostKey /run/mwc-ssh/ssh_host_ed25519_key" \
      "AuthorizedKeysFile /run/mwc-ssh/authorized_keys" \
      "PasswordAuthentication no" \
      "KbdInteractiveAuthentication no" \
      "PermitRootLogin no" \
      "AllowUsers workspace" \
      "PidFile /run/mwc-ssh/sshd.pid" \
      > /etc/workspace-platform/sshd_config
    exec /usr/local/bin/mwc-workspace-bootstrap serve
  ' >/dev/null

ready=false
for _attempt in $(seq 1 30); do
  if docker exec "$container" python3 -c '
import socket
connection = socket.create_connection(("127.0.0.1", 6080), timeout=1)
connection.sendall(b"GET / HTTP/1.0\r\nHost: localhost\r\n\r\n")
assert b" 200 " in connection.recv(200)
'; then
    ready=true
    break
  fi
  sleep 1
done

if [ "$ready" != true ]; then
  echo "Kali browser desktop did not serve noVNC on 127.0.0.1:6080" >&2
  exit 1
fi

docker exec "$container" python3 -c '
from pathlib import Path

listeners = Path("/proc/net/tcp").read_text() + Path("/proc/net/tcp6").read_text()
for port in ("17C0", "170D"):  # 6080 and 5901 in hexadecimal
    assert f"0100007F:{port}" in listeners, listeners
    assert f"00000000:{port}" not in listeners, listeners
    assert f"00000000000000000000000000000000:{port}" not in listeners, listeners
'
docker exec "$container" /bin/sh -ec '
  test "$(stat -c %a /tmp/.X11-unix)" = 1777
  test -s /run/mwc-ssh/desktop.pid
  kill -0 "$(cat /run/mwc-ssh/desktop.pid)"
'

# TigerVNC waits for the desktop session during startup. A missing X11 socket
# directory used to look healthy briefly and then exit at the 30-second mark.
sleep 35
docker exec "$container" python3 -c '
import socket
connection = socket.create_connection(("127.0.0.1", 6080), timeout=2)
connection.sendall(b"GET / HTTP/1.0\r\nHost: localhost\r\n\r\n")
assert b" 200 " in connection.recv(200)
'
docker exec "$container" /bin/sh -ec '
  test -s /run/mwc-ssh/desktop.pid
  kill -0 "$(cat /run/mwc-ssh/desktop.pid)"
'
success=true
