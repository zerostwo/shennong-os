# Shennong OS Agent Runtime

Internal Pi Agent Core service for Shennong OS. It compiles versioned platform and biomedical policies, exposes only server-owned tools, maps Pi lifecycle events to AG-UI SSE events, and delegates durable state and privileged operations to authenticated OS callbacks.

The process is intentionally not a computation sandbox. It has no filesystem, shell, Docker, or arbitrary-network tools. Biomedical computation is submitted to Shennong Runtime through governed callbacks.

## Development

```bash
pnpm install --ignore-scripts
pnpm test
pnpm typecheck
pnpm build
pnpm skills:validate
```

`SHENNONG_AGENT_RUNTIME_SECRET` and `SHENNONG_OS_SERVICE_TOKEN` must each be at least 32 characters. In production, mount them as files and set `SHENNONG_AGENT_RUNTIME_SECRET_FILE` and `SHENNONG_OS_SERVICE_TOKEN_FILE`; direct environment values remain a development fallback. Defining both forms for one secret is rejected. `SHENNONG_OS_INTERNAL_URL` identifies the trusted OS callback base. Internal callback URLs and secrets are configured at process startup and are never accepted from a run request.

The service exposes `POST /v1/agent` and `POST /api/agent` as AG-UI SSE endpoints. `GET /health` is an unauthenticated container health probe and returns no configuration or tenant data.

Build and probe the container with:

```sh
docker build -t shennong-os-agent-runtime:local .
docker run --rm --name shennong-agent-runtime \
  --mount type=bind,src=/srv/shennong.one/secrets/agent-runtime-secret,dst=/run/secrets/agent-runtime-secret,readonly \
  --mount type=bind,src=/srv/shennong.one/secrets/os-service-token,dst=/run/secrets/os-service-token,readonly \
  -e SHENNONG_AGENT_RUNTIME_SECRET_FILE=/run/secrets/agent-runtime-secret \
  -e SHENNONG_OS_SERVICE_TOKEN_FILE=/run/secrets/os-service-token \
  -e SHENNONG_OS_INTERNAL_URL \
  -p 8002:8002 shennong-os-agent-runtime:local
```

Provider traffic is restricted to the OS-authorized public HTTPS base URL, or
to the explicit local-Ollama exception at
`http://host.docker.internal:11434/v1` in the unified deployment. The latter
resolves only inside Agent Runtime and reaches the control-bridge-only systemd
socket proxy documented in `deploy/README.md`; it is not a public provider
exception. Redirects and any other remote hostname resolving to loopback,
RFC1918, link-local, metadata, reserved, or mixed public/private addresses are
rejected.
