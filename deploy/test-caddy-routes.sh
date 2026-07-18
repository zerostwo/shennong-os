#!/usr/bin/env bash
set -Eeuo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
readonly script_dir
readonly caddyfile="$script_dir/Caddyfile"
readonly compose_file="$script_dir/compose.yaml"
caddy_image="${SHENNONG_CADDY_IMAGE:-$(awk '
  $0 == "  gateway:" { in_gateway = 1; next }
  in_gateway && $0 ~ /^  [[:alnum:]_-]+:$/ { exit }
  in_gateway && $1 == "image:" { print $2; exit }
' "$compose_file")}"
readonly caddy_image
[[ "$caddy_image" == *@sha256:* ]] || {
  printf 'Caddy route contract requires a digest-pinned image\n' >&2
  exit 1
}

readonly suffix="$$-${RANDOM}"
readonly network="shennong-caddy-contract-$suffix"
readonly os_server="shennong-caddy-os-$suffix"
readonly web="shennong-caddy-web-$suffix"
readonly gateway="shennong-caddy-gateway-$suffix"
readonly ide_host="ide.contract.test"

cleanup() {
  docker rm --force "$gateway" "$web" "$os_server" >/dev/null 2>&1 || true
  docker network rm "$network" >/dev/null 2>&1 || true
}
trap cleanup EXIT

docker network create "$network" >/dev/null
docker run --detach --name "$os_server" --network "$network" --network-alias os-server \
  "$caddy_image" caddy respond --listen :8080 --body os-server >/dev/null
docker run --detach --name "$web" --network "$network" --network-alias web \
  "$caddy_image" caddy respond --listen :3000 --body web >/dev/null
docker run --detach --name "$gateway" --network "$network" \
  --publish 127.0.0.1::80 \
  --env "SHENNONG_IDE_HOST=$ide_host" \
  --volume "$caddyfile:/etc/caddy/Caddyfile:ro" \
  "$caddy_image" caddy run --config /etc/caddy/Caddyfile --adapter caddyfile >/dev/null

port=$(docker inspect --format '{{(index (index .NetworkSettings.Ports "80/tcp") 0).HostPort}}' "$gateway")
readonly port
readonly base_url="http://127.0.0.1:$port"

for _ in {1..50}; do
  if curl --silent --fail --header 'Host: os.contract.test' "$base_url/" >/dev/null; then
    break
  fi
  sleep 0.1
done

assert_response() {
  local host="$1"
  local path="$2"
  local expected_status="$3"
  local expected_body="$4"
  local response status body
  response="$(curl --silent --show-error --path-as-is --header "Host: $host" --write-out $'\n%{http_code}' "$base_url$path")"
  status="${response##*$'\n'}"
  body="${response%$'\n'*}"
  [[ "$status" == "$expected_status" && "$body" == "$expected_body" ]] || {
    printf 'unexpected Caddy response: host=%s path=%s status=%s body=%q\n' "$host" "$path" "$status" "$body" >&2
    exit 1
  }
}

readonly session_id="00000000-0000-4000-8000-000000000001"
readonly ticket="opaque-ticket-$suffix"
assert_response "$ide_host" "/__shennong/launch?ticket=$ticket" 200 os-server
assert_response "$ide_host" "/v1/sessions/$session_id/proxy/" 200 os-server
assert_response "$ide_host" "/v1/sessions/$session_id/proxy/auth-sign-in?appUri=%2Fs%2Fnotebook" 200 os-server
assert_response "$ide_host" "/v1/sessions/$session_id/proxy/p/abcdef/rstudio/graphics/plot_zoom_png?width=900&height=700" 200 os-server
assert_response "$ide_host" "/v1/sessions/not-a-uuid/proxy/private" 404 ''
assert_response "$ide_host" "/api/v1/auth/session" 404 ''
assert_response os.contract.test "/v1/sessions/$session_id/proxy/private" 200 web

logs=''
for _ in {1..20}; do
  logs="$(docker logs "$gateway" 2>&1)"
  [[ "$logs" == *REDACTED* ]] && break
  sleep 0.1
done
[[ "$logs" != *"$ticket"* ]] || {
  printf 'Caddy access log exposed the raw launch ticket\n' >&2
  exit 1
}
[[ "$logs" == *REDACTED* ]] || {
  printf 'Caddy access log did not show ticket redaction\n' >&2
  exit 1
}

printf 'Caddy IDE route and ticket-redaction contract passed\n'
