#!/bin/sh
set -eu

: "${MWC_CONTROL_PLANE_URL:?}"
: "${MWC_INTERNAL_AUTH_TOKEN_FILE:?}"

test -s /etc/ssh/host-keys/ssh_host_ed25519_key
cp /etc/passwd /run/mwc/passwd.base
cp /etc/group /run/mwc/group
printf 'mwc-workspaces:x:20000:\n' >> /run/mwc/group

sync_users() {
  token=$(cat "$MWC_INTERNAL_AUTH_TOKEN_FILE")
  if ! curl --fail --silent --show-error --max-time 3 \
    --header "Authorization: Bearer ${token}" \
    "${MWC_CONTROL_PLANE_URL}/api/v1/internal/ssh/login-users" \
    > /run/mwc/logins.next; then
    return
  fi
  cp /run/mwc/passwd.base /run/mwc/passwd.next
  uid=20000
  grep -E '^access\+[a-z0-9]{8}$' /run/mwc/logins.next | while IFS= read -r login; do
    printf '%s::%s:20000:Workspace SSH proxy:/nonexistent:/usr/sbin/nologin\n' \
      "$login" "$uid"
    uid=$((uid + 1))
  done >> /run/mwc/passwd.next
  mv /run/mwc/passwd.next /run/mwc/passwd
}

sync_users
(
  while :; do
    sleep 5
    sync_users
  done
) &

export LD_PRELOAD=/usr/local/lib/libnss_wrapper.so
export NSS_WRAPPER_PASSWD=/run/mwc/passwd
export NSS_WRAPPER_GROUP=/run/mwc/group

exec /usr/sbin/sshd -D -e -f /etc/ssh/sshd_config
