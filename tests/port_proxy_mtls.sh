#!/usr/bin/env bash
# Sourced by ttyd_mtls.sh after it prepares the disposable PKI.
: "${image:?}" "${tmp_dir:?}" "${container:?}"

docker rm --force "${container}" >/dev/null
container="${container}-proxy"
ln -s server.crt "${tmp_dir}/tls.crt"
ln -s server.key "${tmp_dir}/tls.key"
ln -s client-ca.crt "${tmp_dir}/ca.crt"
mkdir "${tmp_dir}/routes"
printf '%s\n' 'localhost 3000;' > "${tmp_dir}/routes/ports.conf"
printf '%s\n' \
    'pid /tmp/mwc-origin.pid;' \
    'events {}' \
    'http { access_log off; server { listen 127.0.0.1:3000;' \
    'location / { return 200 "mwc-proxy-ok"; } } }' \
    > "${tmp_dir}/origin.conf"

docker run --detach --rm --name "${container}" --publish 127.0.0.1::8443 \
    --volume "${tmp_dir}:/fixtures:ro" \
    --volume "${tmp_dir}:/etc/mwc-ttyd-tls:ro" \
    --volume "${tmp_dir}/routes:/etc/mwc-http-routes:ro" \
    --entrypoint /bin/sh "${image}" -ec \
    'nginx -c /fixtures/origin.conf; exec nginx -c /etc/mwc-http-proxy.conf -g "daemon off;"' \
    >/dev/null

port=''
for _ in $(seq 1 30); do
    port="$(docker port "${container}" 8443/tcp 2>/dev/null | sed -n 's/.*:\([0-9][0-9]*\)$/\1/p')"
    if [[ -n "${port}" ]] && curl -q --noproxy '*' --silent --fail --max-time 1 \
        --resolve "localhost:${port}:127.0.0.1" \
        --cacert "${tmp_dir}/server-ca.crt" --cert "${tmp_dir}/client.crt" \
        --key "${tmp_dir}/client.key" -H 'Host: localhost' \
        "https://localhost:${port}/" >/dev/null 2>&1; then
        break
    fi
    sleep 1
done
[[ -n "${port}" ]] || fail 'HTTP proxy did not expose a test port'

proxy_request() {
    local address="$1" identity="$2" hostname="$3"
    local -a certificate=()
    case "${identity}" in
        trusted) certificate=(--cert "${tmp_dir}/client.crt" --key "${tmp_dir}/client.key") ;;
        wrong) certificate=(--cert "${tmp_dir}/wrong-client.crt" --key "${tmp_dir}/wrong-client.key") ;;
        none) ;;
        *) fail 'invalid test identity' ;;
    esac
    curl -q --noproxy '*' --silent --show-error --max-time 5 \
        --resolve "localhost:${port}:127.0.0.1" \
        --cacert "${tmp_dir}/server-ca.crt" "${certificate[@]}" \
        -H "Host: ${hostname}" -o "${tmp_dir}/proxy-response" -w '%{http_code}' \
        "https://${address}:${port}/"
}

for address in localhost 127.0.0.1; do
    [[ "$(proxy_request "${address}" trusted localhost)" == 200 ]] \
        || fail 'trusted proxy request failed'
    [[ "$( < "${tmp_dir}/proxy-response")" == mwc-proxy-ok ]] \
        || fail 'proxy returned unexpected upstream content'
    [[ "$(proxy_request "${address}" none localhost)" == 400 ]] \
        || fail 'proxy accepted a missing client certificate'
    [[ "$(proxy_request "${address}" wrong localhost)" == 400 ]] \
        || fail 'proxy accepted an untrusted client certificate'
    [[ "$(proxy_request "${address}" trusted unlisted.example)" == 404 ]] \
        || fail 'proxy accepted an undeclared upstream host'
    [[ "$(proxy_request "${address}" trusted localhost)" == 200 ]] \
        || fail 'proxy did not recover after negative checks'
done
printf '%s\n' 'HTTP proxy mTLS and route allowlist image test passed'
