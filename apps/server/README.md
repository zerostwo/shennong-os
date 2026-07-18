# Shennong OS server

The Rust service is the only browser-facing control plane. It owns user
sessions, project authorization, threads, run state, governed tool grants and
the authenticated streaming gateway to Agent Runtime. Shennong DB and
Shennong Runtime remain private services.

Run events are append-only and exposed to authenticated Project readers through
bounded `after`/`Last-Event-ID` listing and SSE replay/follow endpoints. A
nonzero cursor must belong to the requested Run. Replay is ordered by the
database event id and stops only after delivering a persisted terminal event,
so reconnecting cannot create a second Run.

## Secret files

Production deployments should mount Docker/Swarm/Kubernetes secrets as
read-only files and configure the matching `*_FILE` variable. A plain variable
and its `_FILE` form are mutually exclusive; startup fails if both are set.

| Secret | Preferred production variable |
| --- | --- |
| PostgreSQL URL | `SHENNONG_OS_DATABASE_URL_FILE` |
| First-admin bootstrap token | `SHENNONG_OS_BOOTSTRAP_TOKEN_FILE` |
| Invitation HMAC key | `SHENNONG_OS_INVITE_HMAC_KEY_FILE` |
| Provider credential encryption key | `SHENNONG_OS_PROVIDER_ENCRYPTION_KEY_FILE` |
| Agent Runtime to OS callback token | `SHENNONG_OS_SERVICE_TOKEN_FILE` |
| OS to Agent Runtime token | `SHENNONG_AGENT_RUNTIME_SECRET_FILE` |
| Shennong DB headless admin key | `SHENNONG_DB_ADMIN_KEY_FILE` |
| Shennong Runtime Ed25519 signing key | `SHENNONG_RUNTIME_JWT_ED25519_PRIVATE_KEY_FILE` |

Every secret except the database URL must contain at least 32 characters.
Internal URLs are non-secret configuration: `SHENNONG_AGENT_RUNTIME_URL`,
`SHENNONG_DB_URL`, and `SHENNONG_RUNTIME_URL`. Runtime JWT issuer and audience
default to `shennong-os` and `shennong-runtime` and can be changed with
`SHENNONG_RUNTIME_JWT_ISSUER` and `SHENNONG_RUNTIME_JWT_AUDIENCE`.
The unified production deployment signs with Ed25519 so only OS receives the
private key and Runtime receives the public key. The HS256 file variable is a
compatibility path for an existing deployment and is not used by the V1
Compose profile.

Other important settings are `SHENNONG_OS_ALLOWED_ORIGINS`,
`SHENNONG_PUBLIC_ORIGIN`, `SHENNONG_IDE_PUBLIC_ORIGIN`,
`SHENNONG_OS_COOKIE_SECURE`, `SHENNONG_OS_TRUST_PROXY_HEADERS`, and
`SHENNONG_OS_RUN_MIGRATIONS`. Proxy headers must remain disabled unless the
service is reachable only through a trusted reverse proxy.

`SHENNONG_PUBLIC_ORIGIN` must be present in `SHENNONG_OS_ALLOWED_ORIGINS`.
When Runtime is enabled, `SHENNONG_IDE_PUBLIC_ORIGIN` is required and its host
must differ from the OS host. The IDE host exposes only one-time launch-ticket
redemption and `/v1/sessions/<id>/proxy/**`; ordinary OS APIs are rejected.
Redemption creates a host-only, HttpOnly, SameSite=Strict cookie scoped to one
Runtime Session. The proxy strips every `shennong_*` browser cookie and any
browser Authorization header before sending a fresh 30-second, workspace-bound
Runtime JWT. The reverse proxy must preserve the original `Host`; if trusted
forwarded headers are enabled it must overwrite, rather than append to,
`X-Forwarded-Host`.

The production image runs as UID/GID `65532`, needs no writable filesystem,
and exposes port 8080. Its health check calls `/healthz` and includes the
PostgreSQL readiness probe.

## Verification

```sh
cargo fmt --all -- --check
cargo test --all-targets
TEST_DATABASE_URL=postgres://.../shennong_os_test \
  cargo test --test postgres_integration -- --ignored
TEST_DATABASE_URL=postgres://.../shennong_os_test \
  cargo test --test identity_rbac_integration -- --ignored --test-threads=1
```
