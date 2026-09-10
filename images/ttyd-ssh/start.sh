#!/bin/sh
set -eu

# Kubernetes invokes this wrapper only when the HTTP proxy is configured.
# The image's default entrypoint remains stock ttyd.
nginx -t -q -c /etc/mwc-http-proxy.conf
nginx -c /etc/mwc-http-proxy.conf -g 'daemon off;' &
proxy_pid=$!
ttyd_pid=
cleanup() {
    kill "$proxy_pid" ${ttyd_pid:+"$ttyd_pid"} 2>/dev/null || true
    wait "$proxy_pid" 2>/dev/null || true
    if [ -n "$ttyd_pid" ]; then
        wait "$ttyd_pid" 2>/dev/null || true
    fi
}
trap cleanup EXIT
trap 'exit 0' TERM INT

/usr/bin/ttyd "$@" &
ttyd_pid=$!
routes_revision=
while kill -0 "$proxy_pid" 2>/dev/null && kill -0 "$ttyd_pid" 2>/dev/null; do
    # Projected ConfigMap updates switch the ..data symlink atomically.
    revision=$(readlink /etc/mwc-http-routes/..data 2>/dev/null || true)
    if [ "$revision" != "$routes_revision" ]; then
        if nginx -t -q -c /etc/mwc-http-proxy.conf; then
            kill -HUP "$proxy_pid"
            routes_revision=$revision
        else
            # Never retain an obsolete allowlist after an invalid update.
            exit 1
        fi
    fi
    sleep 2 &
    wait "$!" || true
done
exit 1
