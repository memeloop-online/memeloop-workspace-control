#!/usr/bin/env bash
# Requires Docker and one argument: a locally built ttyd image tag.
set -euo pipefail

if [[ $# -ne 1 ]]; then
    printf '%s\n' 'usage: tests/ttyd_mtls.sh IMAGE_TAG' >&2
    exit 2
fi

image="$1"
container="ttyd-mtls-test-$$"
tmp_dir="$(mktemp -d)"

cleanup() {
    docker rm --force "${container}" >/dev/null 2>&1 || true
    rm -rf "${tmp_dir}"
}
trap cleanup EXIT

fail() {
    printf '%s\n' "ttyd mTLS test: $*" >&2
    docker logs "${container}" >&2 || true
    exit 1
}

openssl req -x509 -newkey rsa:2048 -nodes -days 1 \
    -keyout "${tmp_dir}/server-ca.key" -out "${tmp_dir}/server-ca.crt" \
    -subj '/CN=ttyd-test-server-ca' >/dev/null 2>&1
openssl req -x509 -newkey rsa:2048 -nodes -days 1 \
    -keyout "${tmp_dir}/client-ca.key" -out "${tmp_dir}/client-ca.crt" \
    -subj '/CN=ttyd-test-client-ca' >/dev/null 2>&1
openssl req -newkey rsa:2048 -nodes \
    -keyout "${tmp_dir}/server.key" -out "${tmp_dir}/server.csr" \
    -subj '/CN=localhost' >/dev/null 2>&1
printf '%s\n' 'subjectAltName=DNS:localhost,IP:127.0.0.1' > "${tmp_dir}/server.ext"
openssl x509 -req -days 1 -in "${tmp_dir}/server.csr" \
    -CA "${tmp_dir}/server-ca.crt" -CAkey "${tmp_dir}/server-ca.key" -CAcreateserial \
    -out "${tmp_dir}/server.crt" -extfile "${tmp_dir}/server.ext" >/dev/null 2>&1

make_client() {
    local name="$1"
    local ca_name="$2"
    openssl req -newkey rsa:2048 -nodes \
        -keyout "${tmp_dir}/${name}.key" -out "${tmp_dir}/${name}.csr" \
        -subj "/CN=${name}" >/dev/null 2>&1
    openssl x509 -req -days 1 -in "${tmp_dir}/${name}.csr" \
        -CA "${tmp_dir}/${ca_name}.crt" -CAkey "${tmp_dir}/${ca_name}.key" -CAcreateserial \
        -out "${tmp_dir}/${name}.crt" >/dev/null 2>&1
}

make_client client client-ca
openssl req -x509 -newkey rsa:2048 -nodes -days 1 \
    -keyout "${tmp_dir}/wrong-ca.key" -out "${tmp_dir}/wrong-ca.crt" \
    -subj '/CN=ttyd-wrong-ca' >/dev/null 2>&1
make_client wrong-client wrong-ca

docker run --detach --rm --name "${container}" --publish 127.0.0.1::7681 \
    --volume "${tmp_dir}:/tls:ro" "${image}" \
    --ssl --ssl-cert /tls/server.crt --ssl-key /tls/server.key --ssl-ca /tls/client-ca.crt sh \
    >/dev/null

port=''
for _ in $(seq 1 30); do
    port="$(docker port "${container}" 7681/tcp 2>/dev/null | sed -n 's/.*:\([0-9][0-9]*\)$/\1/p')"
    if [[ -n "${port}" ]] && curl -q --noproxy '*' --silent --show-error --fail --max-time 1 \
        --resolve "localhost:${port}:127.0.0.1" \
        --cacert "${tmp_dir}/server-ca.crt" --cert "${tmp_dir}/client.crt" --key "${tmp_dir}/client.key" \
        "https://localhost:${port}/" >/dev/null 2>&1; then
        break
    fi
    sleep 1
done
[[ -n "${port}" ]] || fail 'container did not become ready'

request() {
    local host="$1"
    shift
    local -a route=()
    if [[ "${host}" == 'localhost' ]]; then
        route=(--resolve "localhost:${port}:127.0.0.1")
    fi
    curl -q --noproxy '*' --silent --show-error --fail --max-time 10 \
        "${route[@]}" --cacert "${tmp_dir}/server-ca.crt" "$@" "https://${host}:${port}/" >/dev/null
}

expect_tls_rejection() {
    local host="$1"
    shift
    local code
    set +e
    request "${host}" "$@" >/dev/null 2>&1
    code=$?
    set -e
    case "${code}" in
        # CURLE_SSL_CONNECT_ERROR, CURLE_SEND_ERROR, CURLE_RECV_ERROR.
        35|55|56) return 0 ;;
        *) fail "expected TLS rejection for ${host}, got curl exit ${code}" ;;
    esac
}

# localhost sends SNI; an IP-literal URL deliberately does not.  The production
# bypass occurred only on the latter branch, so every case covers both.
for host in localhost 127.0.0.1; do
    for tls_mode in default tls12; do
        tls_args=()
        if [[ "${tls_mode}" == tls12 ]]; then
            tls_args=(--tlsv1.2 --tls-max 1.2)
        fi

        # This proves the server CA, SAN, host path, and selected TLS mode work
        # before and after each negative mTLS assertion.
        request "${host}" "${tls_args[@]}" \
            --cert "${tmp_dir}/client.crt" --key "${tmp_dir}/client.key"
        expect_tls_rejection "${host}" "${tls_args[@]}"
        request "${host}" "${tls_args[@]}" \
            --cert "${tmp_dir}/client.crt" --key "${tmp_dir}/client.key"
        expect_tls_rejection "${host}" "${tls_args[@]}" \
            --cert "${tmp_dir}/wrong-client.crt" --key "${tmp_dir}/wrong-client.key"
        request "${host}" "${tls_args[@]}" \
            --cert "${tmp_dir}/client.crt" --key "${tmp_dir}/client.key"
    done
done

printf '%s\n' 'ttyd mTLS image test passed'
