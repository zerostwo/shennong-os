# Unified V1 deployment

This directory deploys the trusted Shennong OS WebUI/control plane, the
headless Shennong DB data plane, the Agent Runtime, PostgreSQL, the trusted
Shennong Runtime daemon, and a single Caddy gateway. Untrusted Job and IDE
containers run in a separate, dedicated rootless Docker daemon.

All application services now default to public `zerostwo/*:latest` Docker Hub
images, so a normal install does not need local builds or seven image variables.
Before starting the stack, pull the trusted images with the system daemon and
the workload images with the dedicated rootless daemon:

```bash
docker compose --env-file /srv/shennong.one/.env \
  --file /srv/shennong.one/compose.yaml pull
DOCKER_HOST=unix:///run/user/<runtime-uid>/shennong-runtime-docker/docker.sock \
  docker pull zerostwo/shennong-runtime-worker:latest
DOCKER_HOST=unix:///run/user/<runtime-uid>/shennong-runtime-docker/docker.sock \
  docker pull zerostwo/shennong-runtime-ide:latest
```

For reproducible production and rollback, override the defaults with immutable
digests in `.env`: `SHENNONG_OS_WEB_IMAGE`, `SHENNONG_OS_SERVER_IMAGE`,
`SHENNONG_AGENT_RUNTIME_IMAGE`, `SHENNONG_DB_IMAGE`,
`SHENNONG_RUNTIME_DAEMON_IMAGE`, `SHENNONG_WORKER_IMAGE`, and
`SHENNONG_IDE_IMAGE`. Caddy and PostgreSQL remain reviewed, digest-pinned
constants in Compose. `SHENNONG_SECRETS_DIR` defaults to `./secrets`, while the
three persistent-data paths default below `/srv/shennong.one/data`.

The gateway publishes one TCP port but uses two browser origins. The product
origin reaches only the WebUI. The independent IDE host reaches only the OS IDE
ticket/proxy routes. DB, Agent Runtime, PostgreSQL, Runtime control, and raw IDE
ports are never published.

Project uploads are streamed WebUI -> OS -> DB with the browser-visible actor
and Project fields discarded. OS injects authenticated UUIDs over the private
service boundary, DB requires the same actor/Project when registering, and the
resulting private Resource is bound atomically to that Project. The default
50 GiB stream cap and four-hour transfer timeout are controlled by
`SHENNONG_MAX_UPLOAD_BYTES` and `SHENNONG_UPLOAD_TIMEOUT_SECONDS`; OS and DB
must use the same byte limit.

## Required host state

- cgroup v2 and rootless Docker prerequisites;
- a dedicated `shennong-runtime` account with subordinate UID/GID ranges and
  linger enabled;
- `/srv/shennong.one/data/runtime` and the dedicated rootless Docker data root
  owned by that account;
- a digest-pinned worker image and IDE image already pulled into that daemon;
- the supplied root-owned nftables egress-policy guard installed and verified
  against the live Runtime control address. Its RootlessKit state and signed-off
  attestation directories are mounted read-only into Runtime so a missing or
  stale policy makes health checks and workload launches fail closed.

If an administrator enables the supported local Ollama connection, install
`systemd/shennong-ollama-proxy.socket` and
`systemd/shennong-ollama-proxy.service` under `/etc/systemd/system/`, then
enable the socket. It binds only the Shennong control bridge gateway
`172.30.0.1:11434` and proxies to the host's loopback Ollama. Compose resolves
`host.docker.internal` only inside Agent Runtime; no Ollama port is published
on the LAN and no untrusted Job or IDE network can reach the control bridge.

Copy `.env.example` to `/srv/shennong.one/.env`, set the two public origins and
the dedicated Runtime account IDs/socket paths, and generate each symmetric
secret under `/srv/shennong.one/secrets` with at least 32 cryptographically
random bytes. Generate the Runtime signing pair separately:

```bash
openssl genpkey -algorithm ED25519 \
  -out /srv/shennong.one/secrets/runtime-jwt-ed25519-private.pem
openssl pkey \
  -in /srv/shennong.one/secrets/runtime-jwt-ed25519-private.pem \
  -pubout \
  -out /srv/shennong.one/secrets/runtime-jwt-ed25519-public.pem
```

Only OS mounts the private key; Runtime mounts only the public key. Never copy
the example origin values unchanged to a public host.
Create a host-only secrets group with the GID configured by
`SHENNONG_SECRETS_GID`; secret files are `root:<gid>` and mode `0640`, while
only the containers that need them receive that supplemental GID. The database
URL file has the form:

```text
postgres://shennong_os:<url-encoded-password>@os-postgres:5432/shennong_os
```

Keep the deployment root and secret directory mode `0700`, each secret `0640`,
and the Runtime state directory owned by the dedicated Runtime UID/GID. Copy
`compose.yaml` and `Caddyfile` from this directory into the deployment root.
Install the three recovery scripts as non-world-readable files; they can be
invoked through `bash` and do not need an executable bit:

```bash
sudo install -d -m 0700 /srv/shennong.one/ops
sudo install -m 0600 deploy/backup-unified.sh \
  deploy/restore-unified.sh deploy/restore-drill.sh \
  /srv/shennong.one/ops/
```

Keep the checked-in examples in source control; keep the populated `.env`,
`secrets/`, runtime state, and generated `versions.env` outside every Git
repository.

The included `rootless-data.mount` is the host-specific unit used by the V1
single-node deployment to mount an 80 GiB filesystem image at the dedicated
daemon data root. Install it under the escaped mount-unit name returned by:

```bash
systemd-escape --path --suffix=mount \
  /home/shennong-runtime/.local/share/shennong-runtime-docker
```

Create and format the backing file only once; formatting an existing file
destroys its contents. Enable the mount before enabling the rootless daemon.

Then validate and start the trusted stack:

```bash
docker compose --env-file /srv/shennong.one/.env \
  --file /srv/shennong.one/compose.yaml config
docker compose --env-file /srv/shennong.one/.env \
  --file /srv/shennong.one/compose.yaml pull
docker compose --env-file /srv/shennong.one/.env \
  --file /srv/shennong.one/compose.yaml up --detach
```

Record the exact image IDs/digests deployed for all three repositories in
`/srv/shennong.one/versions.env`. Tags such as `1.0.0` are convenient build
inputs, but production Compose values and the rollback record should use
immutable repository digests once the images are published.

Start from [`versions.env.example`](versions.env.example), record the exact
`v1.0.0` commit for each repository and every deployed image digest, and keep
the populated file outside Git. It is an operator rollback record, not a
Compose environment file.

## Verification

Run these checks after every install or upgrade. They do not print secret
files or container environments:

```bash
docker compose --env-file /srv/shennong.one/.env \
  --file /srv/shennong.one/compose.yaml config --quiet
docker compose --env-file /srv/shennong.one/.env \
  --file /srv/shennong.one/compose.yaml up --detach --wait
docker compose --env-file /srv/shennong.one/.env \
  --file /srv/shennong.one/compose.yaml ps
curl --fail --silent --show-error \
  "$(sed -n 's/^SHENNONG_PUBLIC_ORIGIN=//p' /srv/shennong.one/.env)/healthz"
```

Also complete the Runtime behavioral egress test, one successful bounded batch
Job, one timeout/cancellation path, both IDE launch/stop flows through the IDE
origin, first-admin bootstrap, invitation registration, and a cross-user RBAC
denial. A healthy container list alone is not a V1 acceptance test.

On an empty OS database the WebUI redirects to the bootstrap form. The first
administrator must present the one-time token stored in
`secrets/os-bootstrap-token`. After bootstrap, registration remains visible but
requires an administrator-issued invitation by default.

For public HTTPS, replace the local origins with two real HTTPS hostnames, set
`SHENNONG_OS_COOKIE_SECURE=true`, terminate TLS at the gateway, and keep the two
hosts distinct.

## Backup scope and consistency

`backup-unified.sh` creates one new directory; it refuses an existing target,
relative or non-canonical paths, `/`, `$HOME`, this source workspace, and any
destination below live data or secrets. The backup directory and every child
directory are mode `0700`; regular files are mode `0600`. Its exact V1 scope is:

Run all three recovery scripts as root: they must read root-owned secrets,
quiesce trusted Compose services, preserve numeric ownership, and reach the
dedicated rootless Docker socket. They never print secret file contents.

| State | Backup method | Restore coverage |
| --- | --- | --- |
| OS PostgreSQL | custom-format `pg_dump`, without owners or ACLs | restored into the fixed `shennong_os` database |
| headless DB `/data` | stopped-service tar with numeric owners, ACLs, and xattrs | exact `/srv/shennong.one/data/db` replacement |
| Runtime journal | stopped-service tar after SQLite `integrity_check` | exact `/srv/shennong.one/data/runtime` replacement |
| deployment record | `.env`, Compose, Caddy, `versions.env`, and `secrets/` | copied back only by explicit replacement restore |
| Project workspaces | no default traversal; only exact allowlisted, label-verified rootless volumes | archive verification only in V1; no automatic volume restore |

The script records image metadata and checksums every payload in
`MANIFEST.sha256`, then writes `BACKUP_COMPLETE`. It temporarily stops only the
currently-running `os-server`, `agent-runtime`, `shennong-db`, and `runtime`
services. PostgreSQL stays up for the logical dump. A trap starts only services
that were running before the backup. The product may return an error during
this quiesce window, so schedule large DB or workspace copies as maintenance.
This is a coordinated single-node snapshot, not a distributed point-in-time
recovery protocol for already-running rootless Jobs.

Create the protected parent once, then name every backup explicitly:

```bash
sudo install -d -m 0700 /srv/shennong.one/storage/backups
sudo bash /srv/shennong.one/ops/backup-unified.sh \
  /srv/shennong.one/storage/backups/unified-$(date -u +%Y%m%dT%H%M%SZ)
```

Backups contain live provider keys, signing material, invitation keys, service
credentials, and database credentials. Mode `0600` protects them only on this
host. The backup is **not encrypted**: encrypt it with an organization-approved
authenticated encryption and key-custody process before any off-host copy, and
never put it in Git, object storage, or removable media in plaintext.

### Explicit Project workspace export

Project workspaces live in the dedicated rootless Docker daemon and are not
silently included. To export selected workspaces, create a root-readable
tab-separated allowlist with exactly `workspace_ref` and its deterministic
volume name on each line:

```text
ws_example_project_001	shennong-ws-0123456789abcdef0123456789abcdef
```

The actual suffix is the first 32 hexadecimal characters of the SHA-256 of the
exact `workspace_ref`; do not copy the illustrative value above. Review both
values against the OS/Runtime records, stop every Job and IDE using them, and
set an exact digest-pinned helper image:

```bash
sudo env \
SHENNONG_WORKSPACE_ALLOWLIST_FILE=/srv/shennong.one/storage/workspace-backup-allowlist.tsv \
SHENNONG_WORKSPACE_BACKUP_IMAGE='registry.example/shennong-runtime-worker@sha256:<64-hex-digest>' \
bash /srv/shennong.one/ops/backup-unified.sh \
  /srv/shennong.one/storage/backups/unified-with-workspaces-$(date -u +%Y%m%dT%H%M%SZ)
```

For every requested volume, the script derives the expected name, uses only the
configured dedicated rootless socket, and requires
`dev.shennong.managed=true`, `kind=workspace-volume`, the exact Runtime instance,
and the exact workspace label. It rejects an in-use volume. It never lists,
backs up, stops, or removes arbitrary volumes or anything from the system
Docker daemon. Workspace tar files are recovery exports only: V1 deliberately
does not manufacture volumes behind Runtime's ownership boundary or claim a
blanket workspace restore. Durable inputs and results should also exist as
governed Project files and immutable Artifacts.

## Restore and disposable drill

Run the drill after every material backup and before relying on it. The drill
validates the exact manifest file set and hashes, safely extracts DB and Runtime
archives below a disposable directory, checks the DB archive structure, runs
SQLite `integrity_check`, validates any allowlisted workspace tar, and restores
the OS dump into a uniquely named, label-owned PostgreSQL container with no
published port. Its trap removes only that exact disposable container and temp
directory. It does not read or mutate `/srv/shennong.one`:

```bash
sudo bash /srv/shennong.one/ops/restore-drill.sh \
  /srv/shennong.one/storage/backups/unified-20260718T120000Z
```

`SHENNONG_DRILL_TMP_ROOT` may select another existing, canonical scratch
filesystem. `SHENNONG_DRILL_POSTGRES_IMAGE` may override the default only with
an immutable repository digest. Ensure the scratch filesystem can hold an
extracted DB and Runtime copy before starting the drill.

Production restore is fail-closed. It supports only the exact
`/srv/shennong.one` V1 layout, requires a running PostgreSQL service and an
already-created deployment, validates the whole backup before stopping a
service, refuses replacement by default, and requires both the target and
replacement acknowledgement:

```bash
sudo env \
ALLOW_REPLACE=1 \
SHENNONG_RESTORE_TARGET=/srv/shennong.one \
bash /srv/shennong.one/ops/restore-unified.sh \
  /srv/shennong.one/storage/backups/unified-20260718T120000Z
```

Before the first production write it automatically makes a fresh checksummed
`storage/backups/before-restore-<UTC>` rollback backup. The previous DB,
Runtime, and secrets directories are also retained with
`.before-restore-<UTC>` suffixes. PostgreSQL is dropped and recreated from the
custom dump inside one restore transaction; DB and Runtime archives are first
validated in staging directories and only then moved to their exact targets.
The restore copies the backed-up secrets and deployment metadata, so use a
backup from the same credential epoch. V1 does not silently reconcile an OS
PostgreSQL role whose password was independently rotated after that backup.
Secret copies remain mode `0600` inside the backup. On the live target the
restore validates the backed-up `SHENNONG_SECRETS_GID`, then reinstates secret
files as `root:<secrets-gid>` mode `0640` and secret directories as
`root:root` mode `0700`, matching the Compose supplemental-group boundary.

Workspace archives are checksum- and structure-validated but are never written
into rootless Docker by the restore tool. After any restore, review the retained
rollback paths, apply the restored Compose definition, and repeat the complete
acceptance gate:

```bash
docker compose --env-file /srv/shennong.one/.env \
  --file /srv/shennong.one/compose.yaml up --detach --wait
docker compose --env-file /srv/shennong.one/.env \
  --file /srv/shennong.one/compose.yaml ps
```

Do not delete the `before-restore` backup or directory snapshots until OS
login/RBAC, DB Resource and Artifact reads, one real batch Job, both IDE kinds,
and the Runtime egress/isolation checks pass.
