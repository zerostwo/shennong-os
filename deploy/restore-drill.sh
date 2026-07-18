#!/usr/bin/env bash
# Non-destructive restore drill for a Shennong unified backup. This script never
# opens, stops, writes, or replaces a production deployment directory.
set -Eeuo pipefail

umask 077

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
readonly SCRIPT_DIR
readonly BACKUP_FORMAT="shennong-unified-v1"
readonly DEFAULT_POSTGRES_IMAGE="postgres:17.10-bookworm@sha256:4f736ae292687621d4dbe0d499ffd024a36bd2ee7d8ca6f2ccd4c800f047b394"

die() {
  printf 'restore-drill: %s\n' "$*" >&2
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
for path in root.rglob("*"):
    if path.is_symlink() or not (path.is_file() or path.is_dir()):
        raise SystemExit(f"backup contains a link or special file: {path.relative_to(root)}")
line_pattern = re.compile(r"^([0-9a-f]{64})  (\./[^\0\r\n]+)$")
expected = {}
for number, line in enumerate((root / "MANIFEST.sha256").read_text(encoding="utf-8").splitlines(), 1):
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
    raise SystemExit("checksum manifest does not exactly describe the backup file set")
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

[[ $# -eq 1 ]] || die "usage: restore-drill.sh /absolute/backup-directory"
[[ -z "${SHENNONG_RESTORE_TARGET:-}" ]] || die "SHENNONG_RESTORE_TARGET is forbidden during a disposable drill"
backup=$(require_exact_directory "$1" "backup directory")
readonly backup

source_repo=$(git -C "$SCRIPT_DIR" rev-parse --show-toplevel 2>/dev/null || true)
if [[ -n "$source_repo" ]]; then
  source_repo=$(realpath -e -- "$source_repo")
  [[ "$backup" != "$source_repo" ]] || die "source workspace is not a backup directory"
fi

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
[[ "$(backup_env_value SHENNONG_DB_DATA_DIR)" == /srv/shennong.one/data/db ]] || die "backup DB path is not the exact V1 target"
[[ "$(backup_env_value SHENNONG_OS_POSTGRES_DIR)" == /srv/shennong.one/data/os-postgres ]] || die "backup PostgreSQL path is not the exact V1 target"
[[ "$(backup_env_value SHENNONG_RUNTIME_STATE_DIR)" == /srv/shennong.one/data/runtime ]] || die "backup Runtime path is not the exact V1 target"
secrets_gid=$(backup_env_value SHENNONG_SECRETS_GID)
[[ "$secrets_gid" =~ ^[1-9][0-9]{0,9}$ && "$secrets_gid" -le 2147483647 ]] || die "backup .env contains an invalid secrets GID"
[[ "$(awk -F= '$1 == "deployment_root" {print substr($0, index($0, "=") + 1)}' "$backup/metadata/backup.env")" == /srv/shennong.one ]] ||
  die "backup metadata was not created for the exact V1 target"
[[ -d "$backup/deployment/secrets" && ! -L "$backup/deployment/secrets" ]] || die "backup secrets directory is missing or unsafe"
if find "$backup/deployment/secrets" -mindepth 1 \( -type l -o \! \( -type f -o -type d \) \) -print -quit | grep -q .; then
  die "backup secrets contain a link or special file"
fi

validate_tar "$backup/shennong-db/data.tar" "Shennong DB"
validate_tar "$backup/runtime/state.tar" "Runtime state"
if [[ -f "$backup/workspaces/allowlist.tsv" ]]; then
  while IFS=$'\t' read -r workspace_ref volume_name extra || [[ -n "$workspace_ref$volume_name$extra" ]]; do
    [[ -z "$workspace_ref" || "$workspace_ref" == \#* ]] && continue
    [[ -z "$extra" ]] || die "workspace allowlist requires exactly two tab-separated fields"
    [[ "$workspace_ref" =~ ^ws_[A-Za-z0-9_-]{5,125}$ ]] || die "invalid workspace_ref in backup allowlist"
    [[ "$volume_name" =~ ^shennong-ws-[0-9a-f]{32}$ ]] || die "invalid workspace volume in backup allowlist"
    expected_volume="shennong-ws-$(printf '%s' "$workspace_ref" | sha256sum | awk '{print substr($1,1,32)}')"
    [[ "$volume_name" == "$expected_volume" ]] || die "workspace allowlist violates the Runtime naming contract"
    require_regular_file "$backup/workspaces/$volume_name.tar" "workspace archive"
    validate_tar "$backup/workspaces/$volume_name.tar" "workspace $workspace_ref"
  done <"$backup/workspaces/allowlist.tsv"
fi

drill_tmp_root=$(require_exact_directory "${SHENNONG_DRILL_TMP_ROOT:-/var/tmp}" "drill temporary root")
readonly drill_tmp_root
drill_dir=$(mktemp -d "$drill_tmp_root/shennong-restore-drill.XXXXXXXX")
readonly drill_dir
readonly postgres_image="${SHENNONG_DRILL_POSTGRES_IMAGE:-$DEFAULT_POSTGRES_IMAGE}"
[[ "$postgres_image" =~ ^[^[:space:]]+@sha256:[0-9a-f]{64}$ ]] ||
  die "drill PostgreSQL image must be an immutable repository digest"
drill_id="$(date -u +%Y%m%dT%H%M%SZ)-$$-$RANDOM"
readonly drill_id
readonly pg_container="shennong-restore-drill-$drill_id"
pg_container_id=""

cleanup() {
  local status=$?
  trap - EXIT INT TERM
  existing_container=$(docker inspect --format '{{.Id}}' "$pg_container" 2>/dev/null || true)
  if [[ -n "$existing_container" ]]; then
    label=$(docker inspect --format '{{index .Config.Labels "dev.shennong.restore-drill"}}' "$existing_container" 2>/dev/null || true)
    if [[ "$label" == "$drill_id" ]]; then
      docker rm --force "$existing_container" >/dev/null 2>&1 || status=1
    else
      printf 'restore-drill: refusing to remove container whose drill label changed: %s\n' "$existing_container" >&2
      status=1
    fi
  fi
  case "$drill_dir" in
    "$drill_tmp_root"/shennong-restore-drill.*) rm -rf -- "$drill_dir" ;;
    *)
      printf 'restore-drill: refusing unsafe temporary cleanup path: %s\n' "$drill_dir" >&2
      status=1
      ;;
  esac
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

mkdir -m 0700 -- "$drill_dir/db" "$drill_dir/runtime"
tar --acls --xattrs --numeric-owner -C "$drill_dir/db" -xpf "$backup/shennong-db/data.tar"
tar --acls --xattrs --numeric-owner -C "$drill_dir/runtime" -xpf "$backup/runtime/state.tar"
require_regular_file "$drill_dir/db/postgresql/PG_VERSION" "DB PostgreSQL version"
require_regular_file "$drill_dir/db/.shennong-secrets" "DB internal secret record"
[[ -d "$drill_dir/db/objects" && -d "$drill_dir/db/tiledb" && -d "$drill_dir/db/work/uploads" ]] ||
  die "DB archive does not contain the V1 data-plane structure"
require_regular_file "$drill_dir/runtime/runtime.db" "Runtime SQLite journal"
python3 - "$drill_dir/runtime/runtime.db" <<'PY'
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

if docker container inspect "$pg_container" >/dev/null 2>&1; then
  die "disposable PostgreSQL container name unexpectedly exists: $pg_container"
fi
drill_password=$(openssl rand -hex 24)
pg_container_id=$(docker run --detach --name "$pg_container" --network none \
  --label "dev.shennong.restore-drill=$drill_id" \
  --security-opt no-new-privileges:true \
  --env POSTGRES_USER=shennong_os \
  --env POSTGRES_DB=shennong_os \
  --env "POSTGRES_PASSWORD=$drill_password" \
  "$postgres_image")
unset drill_password

ready=0
for _ in $(seq 1 60); do
  if docker exec "$pg_container_id" pg_isready --username shennong_os --dbname shennong_os >/dev/null 2>&1; then
    ready=1
    break
  fi
  sleep 1
done
((ready == 1)) || die "disposable PostgreSQL did not become ready"
docker exec -i "$pg_container_id" \
  pg_restore --username shennong_os --dbname shennong_os --exit-on-error \
  --single-transaction --no-owner --no-privileges <"$backup/os/postgres.dump"
table_count=$(docker exec "$pg_container_id" \
  psql --username shennong_os --dbname shennong_os --tuples-only --no-align \
  --command "SELECT count(*) FROM information_schema.tables WHERE table_schema = 'public';")
[[ "$table_count" =~ ^[1-9][0-9]*$ ]] || die "restored OS dump contains no public tables"

printf 'restore drill passed: manifest, OS PostgreSQL dump, DB archive, Runtime SQLite journal'
if [[ -f "$backup/workspaces/allowlist.tsv" ]]; then
  printf ', and allowlisted workspace archives'
fi
printf '\n'
