ARG RUST_IMAGE=rust:1.97-bookworm@sha256:77fac8b98f9f46062bb680b6d25d5bcaabfc400143952ebc572e924bcbedc3fa
ARG NODE_IMAGE=node:24.16.0-bookworm-slim@sha256:2c87ef9bd3c6a3bd4b472b4bec2ce9d16354b0c574f736c476489d09f560a203
ARG CADDY_IMAGE=caddy:2.11.3-alpine@sha256:86deaf5e3d3408a6ccec08fbb79989783dd26e206ae10bcf78a801dc8c9ab794

FROM ${RUST_IMAGE} AS server-build
WORKDIR /build
COPY apps/server/Cargo.toml apps/server/Cargo.lock ./apps/server/
COPY apps/server/src ./apps/server/src
COPY migrations ./migrations
COPY openapi ./openapi
WORKDIR /build/apps/server
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,target=/build/apps/server/target,sharing=locked \
    cargo build --locked --release \
    && install -D -m 0755 target/release/shennong-os-server /out/shennong-os-server

FROM ${NODE_IMAGE} AS web-build
WORKDIR /build
RUN corepack enable && corepack prepare pnpm@10.17.1 --activate
COPY apps/web/package.json apps/web/pnpm-lock.yaml apps/web/.npmrc ./
RUN pnpm install --frozen-lockfile
COPY apps/web ./
RUN pnpm build

FROM ${NODE_IMAGE} AS agent-build
WORKDIR /build
RUN corepack enable && corepack prepare pnpm@10.17.1 --activate
COPY apps/agent-runtime/package.json apps/agent-runtime/pnpm-lock.yaml ./
RUN pnpm install --frozen-lockfile --ignore-scripts
COPY apps/agent-runtime/tsconfig.json ./
COPY apps/agent-runtime/src ./src
RUN pnpm build && pnpm prune --prod

FROM ${CADDY_IMAGE} AS caddy

FROM ${NODE_IMAGE}
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
       bash ca-certificates curl openssl postgresql postgresql-client \
    && rm -rf /var/lib/apt/lists/* /var/lib/postgresql/*/main

COPY --from=caddy /usr/bin/caddy /usr/local/bin/caddy
COPY --from=server-build /out/shennong-os-server /usr/local/bin/shennong-os-server
COPY --from=web-build --chown=node:node /build/.next/standalone /opt/shennong/web
COPY --from=web-build --chown=node:node /build/.next/static /opt/shennong/web/.next/static
COPY --from=agent-build --chown=node:node /build/package.json /opt/shennong/agent/package.json
COPY --from=agent-build --chown=node:node /build/node_modules /opt/shennong/agent/node_modules
COPY --from=agent-build --chown=node:node /build/dist /opt/shennong/agent/dist
COPY deploy/container/Caddyfile /etc/shennong/Caddyfile
COPY deploy/container/entrypoint.sh /usr/local/bin/shennong-os-entrypoint
COPY deploy/container/healthcheck.sh /usr/local/bin/shennong-os-healthcheck
RUN chmod 0555 /usr/local/bin/shennong-os-entrypoint /usr/local/bin/shennong-os-healthcheck \
    && mkdir -p /config /data /opt/shennong \
    && chown node:node /data

ENV SHENNONG_AUTO_INIT=true \
    SHENNONG_CONFIG_DIR=/config \
    SHENNONG_DATA_DIR=/data
VOLUME ["/config", "/data"]
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/shennong-os-entrypoint"]
HEALTHCHECK --interval=10s --timeout=5s --start-period=20s --retries=12 CMD ["/usr/local/bin/shennong-os-healthcheck"]
