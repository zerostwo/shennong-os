#!/usr/bin/env bash
set -euo pipefail

config_dir=${SHENNONG_CONFIG_DIR:-/config}
data_dir=${SHENNONG_DATA_DIR:-/data}
auto_init=${SHENNONG_AUTO_INIT:-true}
postgres_data=$data_dir/postgresql

mkdir -p "$config_dir" "$postgres_data"
chmod 0750 "$config_dir"

random_secret() {
  local path=$1
  if [[ ! -s "$path" ]]; then
    [[ "$auto_init" == "true" ]] || { echo "missing required config file: $path" >&2; exit 1; }
    umask 077
    openssl rand -base64 48 >"$path"
  fi
}

for name in os-bootstrap-token os-invite-hmac-key os-provider-encryption-key db-admin-key agent-runtime-secret os-service-token; do
  random_secret "$config_dir/$name"
done

private_key=$config_dir/runtime-jwt-ed25519-private.pem
public_key=$config_dir/runtime-jwt-ed25519-public.pem
if [[ ! -s "$private_key" || ! -s "$public_key" ]]; then
  [[ "$auto_init" == "true" ]] || { echo "missing Runtime Ed25519 key pair in $config_dir" >&2; exit 1; }
  umask 077
  openssl genpkey -algorithm ED25519 -out "$private_key"
  openssl pkey -in "$private_key" -pubout -out "$public_key"
fi
chgrp -R node "$config_dir"
chmod 0640 "$config_dir"/*

pg_bin=$(pg_config --bindir)
chown -R postgres:postgres "$postgres_data"
if [[ ! -s "$postgres_data/PG_VERSION" ]]; then
  runuser -u postgres -- "$pg_bin/initdb" -D "$postgres_data" --username=shennong_os --auth=trust >/dev/null
fi
runuser -u postgres -- "$pg_bin/pg_ctl" -D "$postgres_data" \
  -o "-c listen_addresses=127.0.0.1 -p 5432" -w start >/dev/null
if ! runuser -u postgres -- psql -U shennong_os -d shennong_os -c 'SELECT 1' >/dev/null 2>&1; then
  runuser -u postgres -- createdb -U shennong_os shennong_os
fi

public_origin=${SHENNONG_PUBLIC_ORIGIN:-http://localhost:8080}
ide_origin=${SHENNONG_IDE_PUBLIC_ORIGIN:-http://ide.localhost:8080}
ide_host=${SHENNONG_IDE_HOST:-ide.localhost}

runuser -u node -- env \
  SHENNONG_AGENT_RUNTIME_HOST=127.0.0.1 \
  SHENNONG_AGENT_RUNTIME_PORT=8002 \
  SHENNONG_AGENT_RUNTIME_SECRET_FILE="$config_dir/agent-runtime-secret" \
  SHENNONG_OS_SERVICE_TOKEN_FILE="$config_dir/os-service-token" \
  SHENNONG_OS_INTERNAL_URL=http://127.0.0.1:8081 \
  NODE_ENV=production \
  node /opt/shennong/agent/dist/index.js &
agent_pid=$!

runuser -u node -- env \
  SHENNONG_OS_BIND=127.0.0.1:8081 \
  SHENNONG_OS_DATABASE_URL=postgres://shennong_os@127.0.0.1:5432/shennong_os \
  SHENNONG_OS_BOOTSTRAP_TOKEN_FILE="$config_dir/os-bootstrap-token" \
  SHENNONG_OS_INVITE_HMAC_KEY_FILE="$config_dir/os-invite-hmac-key" \
  SHENNONG_OS_PROVIDER_ENCRYPTION_KEY_FILE="$config_dir/os-provider-encryption-key" \
  SHENNONG_DB_ADMIN_KEY_FILE="$config_dir/db-admin-key" \
  SHENNONG_RUNTIME_JWT_ED25519_PRIVATE_KEY_FILE="$private_key" \
  SHENNONG_AGENT_RUNTIME_SECRET_FILE="$config_dir/agent-runtime-secret" \
  SHENNONG_OS_SERVICE_TOKEN_FILE="$config_dir/os-service-token" \
  SHENNONG_PUBLIC_ORIGIN="$public_origin" \
  SHENNONG_IDE_PUBLIC_ORIGIN="$ide_origin" \
  SHENNONG_OS_ALLOWED_ORIGINS="${SHENNONG_OS_ALLOWED_ORIGINS:-$public_origin}" \
  SHENNONG_OS_COOKIE_SECURE="${SHENNONG_OS_COOKIE_SECURE:-false}" \
  SHENNONG_OS_TRUST_PROXY_HEADERS=true \
  SHENNONG_OS_RUN_MIGRATIONS=true \
  SHENNONG_DB_URL="${SHENNONG_DB_URL:-http://shennong-db:8000}" \
  SHENNONG_RUNTIME_URL="${SHENNONG_RUNTIME_URL:-http://shennong-runtime:7000}" \
  SHENNONG_AGENT_RUNTIME_URL=http://127.0.0.1:8002/v1/agent \
  RUST_LOG="${RUST_LOG:-info,tower_http=info}" \
  /usr/local/bin/shennong-os-server &
server_pid=$!

runuser -u node -- env \
  NODE_ENV=production \
  HOSTNAME=127.0.0.1 \
  PORT=3000 \
  SHENNONG_API_INTERNAL_URL=http://127.0.0.1:8081 \
  SHENNONG_AGENT_INTERNAL_URL=http://127.0.0.1:8081/api/v1/agent \
  NEXT_PUBLIC_SHENNONG_PUBLIC_URL="$public_origin" \
  node /opt/shennong/web/server.js &
web_pid=$!

SHENNONG_IDE_HOST=$ide_host caddy run --config /etc/shennong/Caddyfile --adapter caddyfile &
caddy_pid=$!

# Invoked indirectly by the signal/EXIT traps below.
# shellcheck disable=SC2317,SC2329
shutdown() {
  trap - INT TERM EXIT
  kill -TERM "$caddy_pid" "$web_pid" "$server_pid" "$agent_pid" 2>/dev/null || true
  wait "$caddy_pid" "$web_pid" "$server_pid" "$agent_pid" 2>/dev/null || true
  runuser -u postgres -- "$pg_bin/pg_ctl" -D "$postgres_data" -m fast stop >/dev/null 2>&1 || true
}
trap shutdown INT TERM EXIT

set +e
wait -n "$caddy_pid" "$web_pid" "$server_pid" "$agent_pid"
status=$?
set -e
exit "$status"
