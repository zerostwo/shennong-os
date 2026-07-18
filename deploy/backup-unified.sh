#!/usr/bin/env bash
# Create one checksummed, host-local Shennong V1 backup. Project workspaces are
# exported only when the operator supplies an exact workspace/volume allowlist.
set -Eeuo pipefail

umask 077

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
readonly SCRIPT_DIR
readonly BACKUP_FORMAT="shennong-unified-v1"
readonly DEFAULT_DEPLOY_ROOT="/srv/shennong.one"
readonly QUIESCE_SERVICES=(os-server agent-runtime shennong-db runtime)

die() {
  printf 'backup-unified: %s\n' "$*" >&2
  exit 1
}

require_regular_file() {
  local path=$1
  local label=$2
  [[ -f "$path" && ! -L "$path" ]] || die "$label must be a regular, non-symlink file: $path"
}

require_exact_directory() {
  local requested=$1
  local label=$2
  local resolved
  [[ "$requested" == /* ]] || die "$label must be an absolute path"
  [[ "$requested" != *'/../'* && "$requested" != */.. && "$requested" != *'/./'* ]] ||
    die "$label must not contain dot path segments"
  [[ -d "$requested" && ! -L "$requested" ]] || die "$label must be an existing, non-symlink directory: $requested"
  resolved=$(realpath -e -- "$requested")
  [[ "$requested" == "$resolved" ]] || die "$label must be its exact canonical path: $resolved"
  printf '%s\n' "$resolved"
}

reject_dangerous_target() {
  local target=$1
  local home_real=""
  local source_repo=""
  [[ "$target" != / ]] || die "refusing filesystem root as the backup destination"
  if [[ -n "${HOME:-}" && -d "$HOME" ]]; then
    home_real=$(realpath -e -- "$HOME")
    [[ "$target" != "$home_real" ]] || die "refusing HOME as the backup destination"
  fi
  source_repo=$(git -C "$SCRIPT_DIR" rev-parse --show-toplevel 2>/dev/null || true)
  if [[ -n "$source_repo" ]]; then
    source_repo=$(realpath -e -- "$source_repo")
    [[ "$target" != "$source_repo" && "$target" != "$source_repo"/* ]] ||
      die "refusing to write a backup inside the source workspace: $source_repo"
  fi
}

require_v1_data_path() {
  local actual=$1
  local expected=$2
  local label=$3
  [[ "$actual" == "$expected" ]] || die "$label must be the reviewed V1 target $expected (got $actual)"
  [[ -d "$actual" && ! -L "$actual" ]] || die "$label must be an existing, non-symlink directory: $actual"
}

contains_line() {
  local needle=$1
  shift
  local value
  for value in "$@"; do
    [[ "$value" == "$needle" ]] && return 0
  done
  return 1
}

readonly requested_deploy_root="${SHENNONG_DEPLOY_ROOT:-$DEFAULT_DEPLOY_ROOT}"
deploy_root=$(require_exact_directory "$requested_deploy_root" "deployment root")
readonly deploy_root
readonly environment_file="$deploy_root/.env"
readonly compose_file="$deploy_root/compose.yaml"
readonly caddy_file="$deploy_root/Caddyfile"
readonly versions_file="$deploy_root/versions.env"
readonly secrets_dir="$deploy_root/secrets"

require_regular_file "$environment_file" "Compose environment"
require_regular_file "$compose_file" "Compose file"
require_regular_file "$caddy_file" "Caddy file"
require_regular_file "$versions_file" "version record"
[[ -d "$secrets_dir" && ! -L "$secrets_dir" ]] || die "secrets must be an existing, non-symlink directory: $secrets_dir"
if find "$secrets_dir" -mindepth 1 \( -type l -o \! \( -type f -o -type d \) \) -print -quit | grep -q .; then
  die "secrets may contain only regular files and directories"
fi

set -a
# The populated deployment .env is trusted root-owned configuration. It is
# sourced only to resolve the three reviewed bind paths and rootless endpoint.
# shellcheck disable=SC1090
source "$environment_file"
set +a

readonly db_data="$deploy_root/data/db"
readonly postgres_data="$deploy_root/data/os-postgres"
readonly runtime_state="$deploy_root/data/runtime"
require_v1_data_path "${SHENNONG_DB_DATA_DIR:-}" "$db_data" "Shennong DB data"
require_v1_data_path "${SHENNONG_OS_POSTGRES_DIR:-}" "$postgres_data" "OS PostgreSQL data"
require_v1_data_path "${SHENNONG_RUNTIME_STATE_DIR:-}" "$runtime_state" "Runtime state"
require_regular_file "$runtime_state/runtime.db" "Runtime SQLite journal"

[[ $# -eq 1 ]] || die "usage: backup-unified.sh /absolute/new/backup-directory"
readonly requested_backup=$1
[[ "$requested_backup" == /* ]] || die "backup destination must be absolute"
[[ "$requested_backup" != *'/../'* && "$requested_backup" != */.. && "$requested_backup" != *'/./'* ]] ||
  die "backup destination must not contain dot path segments"
backup_parent_requested=$(dirname -- "$requested_backup")
readonly backup_parent_requested
backup_parent=$(require_exact_directory "$backup_parent_requested" "backup parent")
readonly backup_parent
backup_dir="$backup_parent/$(basename -- "$requested_backup")"
readonly backup_dir
[[ "$requested_backup" == "$backup_dir" ]] || die "backup destination must be its exact canonical path: $backup_dir"
reject_dangerous_target "$backup_dir"
[[ ! -e "$backup_dir" && ! -L "$backup_dir" ]] || die "backup destination already exists: $backup_dir"
case "$backup_dir" in
  "$db_data"|"$db_data"/*|"$postgres_data"|"$postgres_data"/*|"$runtime_state"|"$runtime_state"/*|"$secrets_dir"|"$secrets_dir"/*)
    die "backup destination must not be inside live data or secrets"
    ;;
esac

readonly -a compose=(docker compose --env-file "$environment_file" --file "$compose_file")
"${compose[@]}" config --quiet
mapfile -t originally_running < <("${compose[@]}" ps --status running --services)
contains_line os-postgres "${originally_running[@]}" || die "os-postgres must be running for a logical backup"

mkdir -m 0700 -- "$backup_dir"
mkdir -m 0700 -- "$backup_dir/os" "$backup_dir/shennong-db" "$backup_dir/runtime" \
  "$backup_dir/deployment" "$backup_dir/metadata" "$backup_dir/workspaces"

declare -a stopped_services=()
services_restarted=0

restart_services() {
  local index
  local failed=0
  if ((services_restarted)); then
    return 0
  fi
  for ((index=${#stopped_services[@]} - 1; index >= 0; index--)); do
    if ! "${compose[@]}" start "${stopped_services[index]}" >/dev/null; then
      printf 'backup-unified: failed to restart %s\n' "${stopped_services[index]}" >&2
      failed=1
    fi
  done
  services_restarted=1
  return "$failed"
}

cleanup() {
  local status=$?
  trap - EXIT INT TERM
  if ! restart_services; then
    status=1
  fi
  if ((status != 0)); then
    printf 'backup-unified: incomplete backup retained for diagnosis (no success marker): %s\n' "$backup_dir" >&2
  fi
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

for service in "${QUIESCE_SERVICES[@]}"; do
  if contains_line "$service" "${originally_running[@]}"; then
    "${compose[@]}" stop --timeout 60 "$service" >/dev/null
    stopped_services+=("$service")
  fi
done

python3 - "$runtime_state/runtime.db" <<'PY'
import sqlite3
import sys

connection = sqlite3.connect(f"file:{sys.argv[1]}?mode=ro", uri=True)
try:
    result = connection.execute("PRAGMA integrity_check").fetchone()
finally:
    connection.close()
if result != ("ok",):
    raise SystemExit(f"Runtime SQLite integrity_check failed: {result!r}")
PY

"${compose[@]}" exec -T os-postgres \
  pg_dump --username shennong_os --dbname shennong_os --format custom \
  --compress 6 --no-owner --no-privileges >"$backup_dir/os/postgres.dump"

tar --acls --xattrs --numeric-owner --sparse -C "$db_data" -cpf \
  "$backup_dir/shennong-db/data.tar" .
tar --acls --xattrs --numeric-owner --sparse -C "$runtime_state" -cpf \
  "$backup_dir/runtime/state.tar" .

workspace_allowlist=${SHENNONG_WORKSPACE_ALLOWLIST_FILE:-}
workspace_count=0
if [[ -n "$workspace_allowlist" ]]; then
  require_regular_file "$workspace_allowlist" "workspace allowlist"
  rootless_socket=${SHENNONG_ROOTLESS_DOCKER_SOCKET:-}
  runtime_uid=${SHENNONG_RUNTIME_UID:-}
  runtime_instance=${SHENNONG_RUNTIME_INSTANCE_ID:-}
  backup_image=${SHENNONG_WORKSPACE_BACKUP_IMAGE:-${SHENNONG_WORKER_IMAGE:-}}
  [[ "$rootless_socket" == "/run/user/$runtime_uid/"* && "$rootless_socket" != /run/docker.sock && "$rootless_socket" != /var/run/docker.sock ]] ||
    die "workspace export requires the dedicated Runtime rootless Docker socket"
  [[ "$runtime_instance" =~ ^[A-Za-z0-9_.-]{3,64}$ ]] || die "invalid Runtime instance id"
  [[ "$backup_image" =~ ^([^[:space:]]+@)?sha256:[0-9a-f]{64}$ ]] ||
    die "SHENNONG_WORKSPACE_BACKUP_IMAGE must be an exact image ID or repository digest"
  readonly -a rootless_docker=(docker --host "unix://$rootless_socket")
  "${rootless_docker[@]}" info --format '{{json .SecurityOptions}}' | grep -q 'name=rootless' ||
    die "configured workspace Docker endpoint is not rootless"
  "${rootless_docker[@]}" image inspect "$backup_image" >/dev/null

  declare -A seen_volumes=()
  while IFS=$'\t' read -r workspace_ref volume_name extra || [[ -n "$workspace_ref$volume_name$extra" ]]; do
    [[ -z "$workspace_ref" || "$workspace_ref" == \#* ]] && continue
    [[ -z "$extra" ]] || die "workspace allowlist requires exactly two tab-separated fields"
    [[ "$workspace_ref" =~ ^ws_[A-Za-z0-9_-]{5,125}$ ]] || die "invalid workspace_ref in allowlist"
    [[ "$volume_name" =~ ^shennong-ws-[0-9a-f]{32}$ ]] || die "invalid workspace volume name in allowlist"
    expected_volume="shennong-ws-$(printf '%s' "$workspace_ref" | sha256sum | awk '{print substr($1,1,32)}')"
    [[ "$volume_name" == "$expected_volume" ]] || die "workspace volume does not match the Runtime naming contract: $workspace_ref"
    [[ -z "${seen_volumes[$volume_name]:-}" ]] || die "duplicate workspace volume in allowlist: $volume_name"
    seen_volumes[$volume_name]=1

    [[ "$("${rootless_docker[@]}" volume inspect --format '{{index .Labels "dev.shennong.managed"}}' "$volume_name")" == true ]] ||
      die "workspace volume is not Runtime-managed: $volume_name"
    [[ "$("${rootless_docker[@]}" volume inspect --format '{{index .Labels "dev.shennong.kind"}}' "$volume_name")" == workspace-volume ]] ||
      die "workspace volume has the wrong managed kind: $volume_name"
    [[ "$("${rootless_docker[@]}" volume inspect --format '{{index .Labels "dev.shennong.instance"}}' "$volume_name")" == "$runtime_instance" ]] ||
      die "workspace volume belongs to another Runtime instance: $volume_name"
    [[ "$("${rootless_docker[@]}" volume inspect --format '{{index .Labels "dev.shennong.workspace_ref"}}' "$volume_name")" == "$workspace_ref" ]] ||
      die "workspace volume label does not match its allowlisted workspace: $volume_name"
    if [[ -n "$("${rootless_docker[@]}" ps --quiet \
      --filter "volume=$volume_name" \
      --filter 'label=dev.shennong.managed=true' \
      --filter "label=dev.shennong.instance=$runtime_instance")" ]]; then
      die "workspace has a running managed container; stop its Job or IDE before export: $workspace_ref"
    fi

    workspace_count=$((workspace_count + 1))
    printf '%s\t%s\n' "$workspace_ref" "$volume_name" >>"$backup_dir/workspaces/allowlist.tsv"
    "${rootless_docker[@]}" run --rm --network none --read-only \
      --cap-drop ALL --security-opt no-new-privileges:true --pids-limit 64 \
      --memory 512m --cpus 1 --user 65532:65532 \
      --label dev.shennong.backup-helper=true \
      --label "dev.shennong.instance=$runtime_instance" \
      --mount "type=volume,src=$volume_name,dst=/workspace,readonly" \
      "$backup_image" tar --numeric-owner --sparse -C /workspace -cpf - . \
      >"$backup_dir/workspaces/$volume_name.tar"
  done <"$workspace_allowlist"
fi
if ((workspace_count == 0)); then
  : >"$backup_dir/workspaces/NO_WORKSPACES_EXPORTED"
fi

cp --archive --no-dereference -- "$compose_file" "$backup_dir/deployment/compose.yaml"
cp --archive --no-dereference -- "$caddy_file" "$backup_dir/deployment/Caddyfile"
cp --archive --no-dereference -- "$environment_file" "$backup_dir/deployment/.env"
cp --archive --no-dereference -- "$versions_file" "$backup_dir/deployment/versions.env"
cp --archive --no-dereference -- "$secrets_dir" "$backup_dir/deployment/secrets"

"${compose[@]}" images --format json >"$backup_dir/metadata/compose-images.json"
{
  printf 'format=%s\n' "$BACKUP_FORMAT"
  printf 'created_at=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  printf 'deployment_root=%s\n' "$deploy_root"
  printf 'runtime_instance_id=%s\n' "${SHENNONG_RUNTIME_INSTANCE_ID:-unknown}"
  printf 'workspace_exports=%s\n' "$workspace_count"
} >"$backup_dir/metadata/backup.env"

find "$backup_dir" -type d -exec chmod 0700 {} +
find "$backup_dir" -type f -exec chmod 0600 {} +
(
  cd -- "$backup_dir"
  find . -type f ! -path ./MANIFEST.sha256 -print0 |
    LC_ALL=C sort -z |
    xargs -0 sha256sum
) >"$backup_dir/MANIFEST.sha256"
chmod 0600 "$backup_dir/MANIFEST.sha256"
printf '%s\n' "$BACKUP_FORMAT" >"$backup_dir/BACKUP_COMPLETE"
chmod 0600 "$backup_dir/BACKUP_COMPLETE"

# BACKUP_COMPLETE is deliberately outside MANIFEST.sha256; its presence means
# every checksummed payload was written. The restore scripts validate both.
restart_services
trap - EXIT INT TERM
printf '%s\n' "$backup_dir"
