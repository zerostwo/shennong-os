# Shennong OS agent guide

This repository owns the Shennong product WebUI, trusted control plane, durable
Agent orchestration, and the unified three-repository deployment contract. Keep
changes inside those boundaries: ShennongDB owns scientific data/provenance and
Shennong Runtime owns isolated code execution.

## Start with repository evidence

1. Read this file, `README.md`, and the relevant part of `ARCHITECTURE.md`.
2. If `.codegraph/` exists, use `codegraph status` and `codegraph explore
   "<symbols or question>"` before `rg`, `find`, or broad file reading. Ask for
   current symbols, call paths, and tests; do not infer behavior from docs alone.
3. If `.codegraph/` is absent, initialize it with `codegraph init .`. Commit only
   `.codegraph/.gitignore`; never commit `codegraph.db`, WAL files, sockets, PIDs,
   logs, or other generated index data.
4. Cross-check behavioral claims against code, `openapi/os-api.yaml`, and
   `deploy/compose.yaml`. Cross-repository claims also require checking the
   matching ShennongDB or Shennong Runtime versioned contract.

When the local environment provides the RTK command proxy, follow its governing
instructions and prefix shell commands with `rtk`.

## Important paths

- `apps/web/`: Next.js WebUI and browser BFF. Browser routes are allowlisted;
  never place service credentials in this package or in `NEXT_PUBLIC_*` values.
- `apps/server/`: Rust/Axum authority for identity, Project RBAC, durable state,
  service credential minting, data-plane mediation, and IDE proxy policy.
- `apps/agent-runtime/`: Pi Agent harness and AG-UI event producer. It must stay
  stateless, filesystem-free, shell-free, and Docker-free; privileged work goes
  through authenticated OS callbacks.
- `migrations/`: append-only OS PostgreSQL schema history.
- `skills/`: immutable versioned built-in Skill content and manifests.
- `openapi/os-api.yaml`: browser- and service-facing OS HTTP contract.
- `deploy/`: production Compose, gateway, secrets, rootless Runtime, backup,
  restore, and smoke-test assets shared by the three-repository deployment.

## Architecture invariants

- OS is the only authority for users, Projects, membership, RBAC, Threads,
  Runs, approvals, Memory, and audit. ShennongDB Project shadows never authorize.
- The browser talks only to the product origin and the restricted IDE origin.
  It never talks directly to DB, Runtime, Agent Runtime, PostgreSQL, or an IDE
  target and never receives a service key or Runtime signing key.
- Authenticate browser requests with session + Origin + CSRF. BFF routes forward
  browser identity through an explicit path/method allowlist; they do not turn a
  browser request into service identity.
- Agent Runtime accepts only OS-authorized Runs, ignores client-supplied tools,
  state, context, and backend URLs, and delegates every governed tool to OS.
- OS talks to headless ShennongDB only after Project RBAC/shadow sync and through
  the exact data allowlist with the DB service key.
- OS signs short-lived, least-scope Runtime JWTs. Only Runtime may receive a
  Docker socket; hardened deployments use its dedicated rootless socket. The
  quick profile's host socket is trusted-single-user only. OS and Agent Runtime
  must never mount either socket.
- Persist Agent events before exposing cursors. Reconnect replays the existing
  Run and must not re-run the model. Approval lineage and tool arguments come
  from durable server state, never from browser assertions.
- Keep secrets out of Git, frontend bundles, URLs, persisted events, and logs.
  Preserve the IDE-host route boundary and launch-ticket redaction.

## Validation

Run the focused checks for the area changed, then the relevant CI-equivalent
group. Do not claim checks passed if dependencies or services were unavailable.

```bash
# Rust control plane
cd apps/server
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets

# Agent Runtime
cd apps/agent-runtime
pnpm install --frozen-lockfile --ignore-scripts
pnpm test
pnpm typecheck
pnpm skills:validate
pnpm build

# WebUI
cd apps/web
pnpm install --frozen-lockfile
pnpm test
pnpm typecheck
pnpm lint
pnpm build

# Contracts and deployment (from repository root)
openapi-spec-validator openapi/os-api.yaml
docker compose --env-file deploy/.env.example --file deploy/compose.yaml config --quiet
bash -n deploy/backup-unified.sh deploy/restore-unified.sh deploy/restore-drill.sh deploy/test-caddy-routes.sh
bash deploy/test-caddy-routes.sh
```

Integration tests that need PostgreSQL use `TEST_DATABASE_URL`; never point them
at production. Deployment or restore work additionally follows `deploy/README.md`
and must preserve existing data with a verified rollback artifact.

## Documentation and release records

- Keep `README.md` as the concise entry point with a rendered Mermaid overview
  and links to the detailed architecture, API, and deployment contracts.
- Update `ARCHITECTURE.md` when component ownership, control/data flow, trust
  boundaries, deployment topology, failure semantics, non-goals, or cross-repo
  contracts change. Verify every claim against current code and configuration.
- Maintain `CHANGELOG.md` in Keep a Changelog 1.1.0 format under `Unreleased`.
  Use `Added`, `Changed`, `Deprecated`, `Removed`, `Fixed`, and `Security` as
  applicable; preserve released entries and version comparison links.
- Use SemVer per repository. Do not describe uncommitted or unreleased behavior
  as released, and do not force OS, DB, and Runtime to share one version.
- API behavior changes require synchronized handler, test, OpenAPI, architecture,
  and changelog updates. Security-boundary changes require explicit negative
  tests, not prose alone.
