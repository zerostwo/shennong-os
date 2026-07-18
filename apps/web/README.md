# Shennong OS WebUI

The unified Next.js interface for Shennong. It uses assistant-ui with the AG-UI
runtime for Project-scoped conversations and provides authenticated access
to Resources, Projects, batch jobs, RStudio Server, JupyterLab, Memory, Skills,
provider settings, invitations, and administration.

The browser only calls same-origin `/api/v1/*` and `/api/agent` routes. These
server-side BFF routes enforce a fixed path/method allowlist and forward cookies,
CSRF tokens, the explicit Project id for Agent requests, and bounded request
bodies to the Shennong OS control plane. Internal Agent callback routes and IDE
proxy routes are not exposed by the generic browser BFF, and DB, Runtime, Agent
Runtime, and service credentials never reach client code.

In-flight conversations use assistant-ui's history `resume()` contract. The
adapter consumes the OS durable Run-event SSE endpoint, remembers the last
applied cursor across transport reconnects, drops an acknowledged boundary
frame if a proxy repeats it, and follows the existing Run to its terminal event;
it never calls the new-Run Agent gateway during resume.

Durable `RUN_FINISHED` interrupt outcomes are restored as assistant-ui native
`requires-action` metadata. The native interrupt hook submits the exact
resolved or cancelled response to `/api/agent`; OS derives the immutable parent
Run and creates the child continuation. The browser never supplies approved
tool arguments or an execution token.

## Development

```bash
corepack enable
pnpm install --frozen-lockfile
pnpm test
pnpm typecheck
pnpm lint
pnpm build
pnpm playwright
```

Required production configuration:

- `SHENNONG_API_INTERNAL_URL`: Shennong OS server base URL.
- `SHENNONG_AGENT_INTERNAL_URL`: authenticated Shennong OS Agent Gateway URL.
- `NEXT_PUBLIC_SHENNONG_PUBLIC_URL`: canonical public origin.

The accepted visual baseline is the migrated ShennongDB Agent-first interface;
V1 changes product ownership and workflow integration without replacing its
layout or design language.
