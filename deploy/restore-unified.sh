#!/usr/bin/env bash
# Restore the trusted Shennong V1 state from one verified unified backup.
# Project workspace archives are intentionally not restored by this V1 tool.
set -Eeuo pipefail

umask 077

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
readonly SCRIPT_DIR
readonly BACKUP_FORMAT="shennong-unified-v1"
readonly QUIESCE_SERVICES=(os-server agent-runtime shennong-db runtime)

die() {
  printf 'restore-unified: %s\n' "$*" >&2
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
  [[ "$resolved" != / ]] || die "refusing filesystem root as $label"
  if [[ -n "${HOME:-}" && -d "$HOME" ]]; then
    [[ "$resolved" != "$(realpath -e -- "$HOME")" ]] || die "refusing HOME as $label"
  fi
  printf '%s\n' "$resolved"
}

verify_backup() {
  local candidate_backup=$1
  require_regular_file "$candidate_backup/MANIFEST.sha256" "checksum manifest"
  require_regular_file "$candidate_backup/BACKUP_COMPLETE" "backup completion marker"
  [[ "$(<"$candidate_backup/BACKUP_COMPLETE")" == "$BACKUP_FORMAT" ]] || die "unsupported or incomplete backup"
  python3 - "$candidate_backup" "$BACKUP_FORMAT" <<'PY'
import hashlib
from pathlib import Path, PurePosixPath
import re
import sys

root = Path(sys.argv[1]).resolve(strict=True)
expected_format = sys.argv[2]
manifest_path = root / "MANIFEST.sha256"
for path in root.rglob("*"):
    if path.is_symlink() or not (path.is_file() or path.is_dir()):
        raise SystemExit(f"backup contains a link or special file: {path.relative_to(root)}")
line_pattern = re.compile(r"^([0-9a-f]{64})  (\./[^\0\r\n]+)$")
expected = {}
for number, line in enumerate(manifest_path.read_text(encoding="utf-8").splitlines(), 1):
    match = line_pattern.fullmatch(line)
    if match is None:
        raise SystemExit(f"unsafe checksum manifest line {number}")
    relative_text = match.group(2)[2:]
    relative = PurePosixPath(relative_text)
    if relative.is_absolute() or not relative.parts or ".." in relative.parts or "." in relative.parts:
        raise SystemExit(f"unsafe checksum path on line {number}")
    if relative_text in expected:
        raise SystemExit(f"duplicate checksum path: {relative_text}")
    candidate = root.joinpath(*relative.parts)
    if candidate.is_symlink() or not candidate.is_file():
        raise SystemExit(f"manifest entry is not a regular file: {relative_text}")
    expected[relative_text] = match.group(1)

actual = {
    path.relative_to(root).as_posix()
    for path in root.rglob("*")
    if path.is_file()
    and path.relative_to(root).as_posix() not in {"MANIFEST.sha256", "BACKUP_COMPLETE"}
}
if actual != set(expected):
    missing = sorted(actual - set(expected))
    extra = sorted(set(expected) - actual)
    raise SystemExit(f"checksum manifest file-set mismatch; unlisted={missing}, missing={extra}")
for relative_text, wanted in expected.items():
    digest = hashlib.sha256()
    with (root / relative_text).open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    if digest.hexdigest() != wanted:
        raise SystemExit(f"checksum mismatch: {relative_text}")

metadata = (root / "metadata" / "backup.env").read_text(encoding="utf-8").splitlines()
if f"format={expected_format}" not in metadata:
    raise SystemExit("backup metadata format mismatch")
PY
}

validate_tar() {
  local archive=$1
  local label=$2
  python3 - "$archive" "$label" <<'PY'
from pathlib import PurePosixPath
import sys
import tarfile

archive, label = sys.argv[1:]
with tarfile.open(archive, "r:*") as handle:
    members = handle.getmembers()
    if not members:
        raise SystemExit(f"{label} archive is empty")
    member_paths = set()
    for member in members:
        path = PurePosixPath(member.name)
        if path.is_absolute() or ".." in path.parts:
            raise SystemExit(f"unsafe path in {label} archive: {member.name!r}")
        member_paths.add(tuple(part for part in path.parts if part not in ("", ".")))
        if member.ischr() or member.isblk() or member.isfifo():
            raise SystemExit(f"special file in {label} archive: {member.name!r}")

    def resolve_inside(parts):
        resolved = []
        for part in parts:
            if part in ("", "."):
                continue
            if part == "..":
                if not resolved:
                    return None
                resolved.pop()
            else:
                resolved.append(part)
        return tuple(resolved)

    for member in members:
        if member.issym() or member.islnk():
            target = PurePosixPath(member.linkname)
            if target.is_absolute():
                raise SystemExit(f"unsafe link in {label} archive: {member.name!r}")
            member_path = tuple(part for part in PurePosixPath(member.name).parts if part not in ("", "."))
            target_parts = (*member_path[:-1], *target.parts) if member.issym() else target.parts
            resolved_target = resolve_inside(target_parts)
            if resolved_target is None or resolved_target not in member_paths:
                raise SystemExit(f"unsafe link in {label} archive: {member.name!r}")
PY
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

[[ $# -eq 1 ]] || die "usage: ALLOW_REPLACE=1 SHENNONG_RESTORE_TARGET=/srv/shennong.one restore-unified.sh /absolute/backup-directory"
[[ "${ALLOW_REPLACE:-0}" == 1 ]] || die "refusing replacement; set ALLOW_REPLACE=1 after reviewing the exact backup and target"
[[ -n "${SHENNONG_RESTORE_TARGET:-}" ]] || die "SHENNONG_RESTORE_TARGET must explicitly name the deployment root"

backup=$(require_exact_directory "$1" "backup directory")
readonly backup
target=$(require_exact_directory "$SHENNONG_RESTORE_TARGET" "restore target")
readonly target
[[ "$target" == /srv/shennong.one ]] || die "V1 restore target must be exactly /srv/shennong.one"

source_repo=$(git -C "$SCRIPT_DIR" rev-parse --show-toplevel 2>/dev/null || true)
if [[ -n "$source_repo" ]]; then
  source_repo=$(realpath -e -- "$source_repo")
  [[ "$target" != "$source_repo" ]] || die "refusing to restore over the source workspace"
fi
[[ "$backup" != "$target" && "$backup" != "$target/data" && "$backup" != "$target/secrets" ]] ||
  die "backup directory must not be a live deployment target"

verify_backup "$backup"
readonly required_backup_files=(
  os/postgres.dump
  shennong-db/data.tar
  runtime/state.tar
  deployment/compose.yaml
  deployment/Caddyfile
  deployment/.env
  deployment/versions.env
  metadata/backup.env
)
for relative in "${required_backup_files[@]}"; do
  require_regular_file "$backup/$relative" "backup payload"
done
backup_env_value() {
  local key=$1
  local value
  value=$(awk -F= -v wanted="$key" '$1 == wanted {print substr($0, index($0, "=") + 1)}' "$backup/deployment/.env")
  [[ -n "$value" && "$value" != *$'\n'* ]] || die "backup .env must contain exactly one literal $key value"
  printf '%s\n' "$value"
}
[[ "$(backup_env_value SHENNONG_DB_DATA_DIR)" == "$target/data/db" ]] || die "backup DB path is not the exact V1 target"
[[ "$(backup_env_value SHENNONG_OS_POSTGRES_DIR)" == "$target/data/os-postgres" ]] || die "backup PostgreSQL path is not the exact V1 target"
[[ "$(backup_env_value SHENNONG_RUNTIME_STATE_DIR)" == "$target/data/runtime" ]] || die "backup Runtime path is not the exact V1 target"
[[ "$(awk -F= '$1 == "deployment_root" {print substr($0, index($0, "=") + 1)}' "$backup/metadata/backup.env")" == "$target" ]] ||
  die "backup metadata was not created for the exact V1 target"
[[ -d "$backup/deployment/secrets" && ! -L "$backup/deployment/secrets" ]] || die "backup secrets directory is missing or unsafe"
if find "$backup/deployment/secrets" -mindepth 1 \( -type l -o \! \( -type f -o -type d \) \) -print -quit | grep -q .; then
  die "backup secrets contain a link or special file"
fi
restored_secrets_gid=$(backup_env_value SHENNONG_SECRETS_GID)
readonly restored_secrets_gid
[[ "$restored_secrets_gid" =~ ^[1-9][0-9]{0,9}$ && "$restored_secrets_gid" -le 2147483647 ]] ||
  die "backup .env must contain one valid SHENNONG_SECRETS_GID"
validate_tar "$backup/shennong-db/data.tar" "Shennong DB"
validate_tar "$backup/runtime/state.tar" "Runtime state"

readonly environment_file="$target/.env"
readonly compose_file="$target/compose.yaml"
readonly db_data="$target/data/db"
readonly postgres_data="$target/data/os-postgres"
readonly runtime_state="$target/data/runtime"
require_regular_file "$environment_file" "live Compose environment"
require_regular_file "$compose_file" "live Compose file"
[[ -d "$target/secrets" && ! -L "$target/secrets" ]] || die "live secrets target is missing or unsafe: $target/secrets"
if find "$target/secrets" -mindepth 1 \( -type l -o \! \( -type f -o -type d \) \) -print -quit | grep -q .; then
  die "live secrets contain a link or special file"
fi
[[ -d "$db_data" && ! -L "$db_data" ]] || die "live DB data target is missing or unsafe: $db_data"
[[ -d "$postgres_data" && ! -L "$postgres_data" ]] || die "live PostgreSQL target is missing or unsafe: $postgres_data"
[[ -d "$runtime_state" && ! -L "$runtime_state" ]] || die "live Runtime target is missing or unsafe: $runtime_state"

set -a
# shellcheck disable=SC1090
source "$environment_file"
set +a
[[ "${SHENNONG_DB_DATA_DIR:-}" == "$db_data" ]] || die "live DB path is not the exact reviewed V1 target"
[[ "${SHENNONG_OS_POSTGRES_DIR:-}" == "$postgres_data" ]] || die "live PostgreSQL path is not the exact reviewed V1 target"
[[ "${SHENNONG_RUNTIME_STATE_DIR:-}" == "$runtime_state" ]] || die "live Runtime path is not the exact reviewed V1 target"

install -d -m 0700 -- "$target/storage" "$target/storage/backups"
timestamp=$(date -u +%Y%m%dT%H%M%SZ)
readonly timestamp
readonly rollback_backup="$target/storage/backups/before-restore-$timestamp"
[[ ! -e "$rollback_backup" ]] || die "before-restore backup target already exists: $rollback_backup"

# Validate and extract into disposable directories before any production write.
stage=$(mktemp -d "$target/storage/.restore-stage.XXXXXXXX")
readonly stage
remove_stage() {
  local status=$?
  trap - EXIT INT TERM
  case "$stage" in
    "$target"/storage/.restore-stage.*) rm -rf -- "$stage" ;;
    *)
      printf 'restore-unified: refusing unsafe staging cleanup path: %s\n' "$stage" >&2
      status=1
      ;;
  esac
  exit "$status"
}
trap remove_stage EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
mkdir -m 0700 -- "$stage/db" "$stage/runtime"
tar --acls --xattrs --numeric-owner -C "$stage/db" -xpf "$backup/shennong-db/data.tar"
tar --acls --xattrs --numeric-owner -C "$stage/runtime" -xpf "$backup/runtime/state.tar"
cp --archive --no-dereference -- "$backup/deployment/secrets" "$stage/secrets"
require_regular_file "$stage/db/postgresql/PG_VERSION" "restored DB PostgreSQL version"
require_regular_file "$stage/db/.shennong-secrets" "restored DB internal secret record"
[[ -d "$stage/db/objects" && -d "$stage/db/tiledb" && -d "$stage/db/work/uploads" ]] ||
  die "restored DB archive does not contain the V1 data-plane structure"
require_regular_file "$stage/runtime/runtime.db" "restored Runtime SQLite journal"
python3 - "$stage/runtime/runtime.db" <<'PY'
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

readonly -a compose=(docker compose --env-file "$environment_file" --file "$compose_file")
"${compose[@]}" config --quiet

# A fresh, checksummed rollback backup is mandatory and is made before any
# restore-side service stop or data replacement. Workspaces are not traversed.
env -u SHENNONG_WORKSPACE_ALLOWLIST_FILE \
  SHENNONG_DEPLOY_ROOT="$target" \
  bash "$SCRIPT_DIR/backup-unified.sh" "$rollback_backup" >/dev/null

mapfile -t originally_running < <("${compose[@]}" ps --status running --services)
contains_line os-postgres "${originally_running[@]}" || die "os-postgres must be running for logical restore"
declare -a stopped_services=()
services_restarted=0
deployment_replaced=0

restart_services() {
  local index
  local failed=0
  if ((services_restarted)); then
    return 0
  fi
  if ((deployment_replaced)); then
    # `compose start` cannot apply the restored images, environment, mounts, or
    # gateway configuration. Reconcile exactly the services that were running
    # before the restore, and wait for their health checks before declaring the
    # replacement successful.
    if ! "${compose[@]}" up --detach --no-deps --wait --wait-timeout 180 \
      "${originally_running[@]}" >/dev/null; then
      printf 'restore-unified: failed to reconcile the restored deployment\n' >&2
      failed=1
    fi
  else
    for ((index=${#stopped_services[@]} - 1; index >= 0; index--)); do
      if ! "${compose[@]}" start "${stopped_services[index]}" >/dev/null; then
        printf 'restore-unified: failed to restart %s\n' "${stopped_services[index]}" >&2
        failed=1
      fi
    done
  fi
  services_restarted=1
  return "$failed"
}

cleanup() {
  local status=$?
  trap - EXIT INT TERM
  if ! restart_services; then
    status=1
  fi
  case "$stage" in
    "$target"/storage/.restore-stage.*) rm -rf -- "$stage" ;;
    *) status=1 ;;
  esac
  if ((status != 0)); then
    printf 'restore-unified: restore failed; rollback backup is preserved at %s\n' "$rollback_backup" >&2
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

"${compose[@]}" exec -T os-postgres \
  dropdb --username shennong_os --if-exists --force shennong_os
"${compose[@]}" exec -T os-postgres \
  createdb --username shennong_os --owner shennong_os shennong_os
"${compose[@]}" exec -T os-postgres \
  pg_restore --username shennong_os --dbname shennong_os --exit-on-error \
  --single-transaction --no-owner --no-privileges <"$backup/os/postgres.dump"

readonly previous_db="$db_data.before-restore-$timestamp"
readonly previous_runtime="$runtime_state.before-restore-$timestamp"
readonly previous_secrets="$target/secrets.before-restore-$timestamp"
[[ ! -e "$previous_db" && ! -e "$previous_runtime" && ! -e "$previous_secrets" ]] ||
  die "a before-restore target already exists for timestamp $timestamp"
mv -- "$db_data" "$previous_db"
mv -- "$stage/db" "$db_data"
mv -- "$runtime_state" "$previous_runtime"
mv -- "$stage/runtime" "$runtime_state"

mv -- "$target/secrets" "$previous_secrets"
mv -- "$stage/secrets" "$target/secrets"
install -m 0600 -- "$backup/deployment/.env" "$target/.env"
install -m 0600 -- "$backup/deployment/compose.yaml" "$target/compose.yaml"
install -m 0600 -- "$backup/deployment/Caddyfile" "$target/Caddyfile"
install -m 0600 -- "$backup/deployment/versions.env" "$target/versions.env"
deployment_replaced=1
find "$target/secrets" -type d -exec chown root:root {} +
find "$target/secrets" -type d -exec chmod 0700 {} +
find "$target/secrets" -type f -exec chown "root:$restored_secrets_gid" {} +
find "$target/secrets" -type f -exec chmod 0640 {} +

restart_services
trap - EXIT INT TERM
case "$stage" in
  "$target"/storage/.restore-stage.*) rm -rf -- "$stage" ;;
  *) die "refusing unsafe staging cleanup path: $stage" ;;
esac
printf 'restore complete; rollback backup: %s\n' "$rollback_backup"
printf 'live DB and Runtime directories also preserved as:\n  %s\n  %s\n' "$previous_db" "$previous_runtime"
if [[ -f "$backup/workspaces/allowlist.tsv" ]]; then
  printf 'workspace archives were verified by checksum but intentionally not restored; see deploy/README.md\n'
fi
