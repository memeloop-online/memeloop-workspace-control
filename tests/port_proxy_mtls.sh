#!/usr/bin/env bash
# Sourced by ttyd_mtls.sh after it prepares the disposable PKI.
: "${image:?}" "${tmp_dir:?}" "${container:?}"

docker rm --force "${container}" >/dev/null
container="${container}-proxy"
ln -s server.crt "${tmp_dir}/tls.crt"
ln -s server.key "${tmp_dir}/tls.key"
ln -s client-ca.crt "${tmp_dir}/ca.crt"
mkdir -p "${tmp_dir}/routes/revision-1"
mapping_host='p-00000000000000000000000000000000.ports.example.invalid'
printf '%s\n' 'localhost 3000;' "\"${mapping_host}\" 3000;" > "${tmp_dir}/routes/revision-1/ports.conf"
ln -s revision-1 "${tmp_dir}/routes/..data"
ln -s ..data/ports.conf "${tmp_dir}/routes/ports.conf"
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
    'nginx -c /fixtures/origin.conf; exec /usr/local/bin/mwc-ttyd-start --port 7681 sh' \
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
    [[ "$(proxy_request "${address}" trusted "${mapping_host}")" == 200 ]] \
        || fail 'proxy rejected a production-length mapping hostname'
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

# Exercise the production supervisor repeatedly: the old initial HUP killed
# nginx before its handlers were installed, and cleanup then killed ttyd.
for _ in $(seq 1 5); do
    docker restart "${container}" >/dev/null
    sleep 3
    port="$(docker port "${container}" 8443/tcp | sed -n 's/.*:\([0-9][0-9]*\)$/\1/p')"
    [[ "$(docker inspect -f '{{.State.Running}}' "${container}")" == true ]] \
        || fail 'Web Shell supervisor exited during cold startup'
    [[ "$(proxy_request localhost trusted localhost)" == 200 ]] \
        || fail 'HTTP proxy failed after supervisor startup'
done

mkdir "${tmp_dir}/routes/revision-2"
printf '%s\n' 'localhost 3000;' > "${tmp_dir}/routes/revision-2/ports.conf"
ln -s revision-2 "${tmp_dir}/routes/..data-new"
mv -Tf "${tmp_dir}/routes/..data-new" "${tmp_dir}/routes/..data"
# nginx briefly drains the old worker generation during a graceful reload.
# Require stable revocation across new connections, rather than one lucky response.
revoked=0
for _ in $(seq 1 15); do
    if [[ "$(proxy_request localhost trusted "${mapping_host}")" == 404 ]]; then
        revoked=$((revoked + 1))
    else
        revoked=0
    fi
    [[ "${revoked}" -ge 3 ]] && break
    sleep 1
done
[[ "${revoked}" -ge 3 ]] || fail 'route revocation did not stabilize' 
[[ "$(proxy_request localhost trusted "${mapping_host}")" == 404 ]] \
    || fail 'supervisor kept a revoked HTTP route'
[[ "$(proxy_request localhost trusted localhost)" == 200 ]] \
    || fail 'route reload interrupted the retained upstream'
printf '%s\n' 'Web Shell supervisor startup and projected route reload test passed'
