#!/usr/bin/env bash
set -euo pipefail

log_dir=${E2E_LOG_DIR:?E2E_LOG_DIR is required}
mkdir -p "$log_dir"
chmod 700 "$log_dir"
fixture_dir=$(mktemp -d "${RUNNER_TEMP:?RUNNER_TEMP is required}/mwc-user-credentials-e2e.XXXXXX")
chmod 700 "$fixture_dir"

cleanup() {
  if [[ -n "${server_pid:-}" ]]; then
    kill "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true
  fi
  rm -rf "$fixture_dir"
}
trap cleanup EXIT

binary=target/release/memeloop-workspace-control
database_url="sqlite://${fixture_dir}/control-plane.sqlite?mode=rwc"
admin_token=$(openssl rand -hex 32)
target_token=$(openssl rand -hex 32)

run_control_plane() {
  env -u KUBECONFIG -u KUBERNETES_SERVICE_HOST -u KUBERNETES_PORT \
    -u MWC_ENCRYPTION_KEY -u MWC_INTERNAL_AUTH_TOKEN \
    MWC_KUBERNETES_ENABLED=0 \
    MWC_INSTALLATION_ID=ci-credentials \
    MWC_LISTEN_ADDRESS=127.0.0.1:18080 \
    MWC_DATABASE_URL="$database_url" \
    MWC_INSTANCE_ID=ci-credentials \
    "$binary" "$@"
}

MWC_ADMIN_TOKEN="$admin_token" run_control_plane admin create-user \
  --display-name "CI Credentials Administrator" --system-admin \
  >"$log_dir/bootstrap.log" 2>&1

run_control_plane >"$log_dir/backend.log" 2>&1 &
server_pid=$!
for attempt in $(seq 1 60); do
  if curl --fail --silent http://127.0.0.1:18080/readyz > /dev/null; then
    break
  fi
  if ! kill -0 "$server_pid" 2>/dev/null; then
    cat "$log_dir/backend.log"
    exit 1
  fi
  sleep 1
done
curl --fail --silent http://127.0.0.1:18080/readyz > /dev/null

organization_json=$(curl --fail --silent --show-error \
  --header "Authorization: Bearer $admin_token" \
  --header "Content-Type: application/json" \
  --header "Idempotency-Key: ci-credentials-organization" \
  --data '{"name":"CI Credentials Organization","owner_user_id":"00000000-0000-0000-0000-000000000000"}' \
  http://127.0.0.1:18080/api/v1/organizations)
organization_id=$(jq --exit-status --raw-output '.id' <<<"$organization_json")

target_json=$(curl --fail --silent --show-error \
  --header "Authorization: Bearer $admin_token" \
  --header "Content-Type: application/json" \
  --header "Idempotency-Key: ci-credentials-target-user" \
  --data "{\"display_name\":\"CI Credentials Target\",\"token\":\"$target_token\",\"organization_id\":\"$organization_id\",\"organization_role\":\"member\"}" \
  http://127.0.0.1:18080/api/v1/admin/users)
target_user_id=$(jq --exit-status --raw-output '.id' <<<"$target_json")

(
  cd web
  E2E_ALLOW_WRITES=1 \
    E2E_LOCAL_BACKEND=1 \
    E2E_BASE_URL=http://127.0.0.1:18080 \
    E2E_ADMIN_TOKEN="$admin_token" \
    E2E_ORGANIZATION_ID="$organization_id" \
    E2E_TARGET_USER_ID="$target_user_id" \
    node --test --test-name-pattern="production administrator manages a specific user's credentials" \
    e2e/production-user-credentials.test.mjs
) 2>&1 | tee "$log_dir/e2e.log"
