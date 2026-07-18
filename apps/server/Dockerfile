FROM rust:1.97-bookworm@sha256:77fac8b98f9f46062bb680b6d25d5bcaabfc400143952ebc572e924bcbedc3fa AS build
WORKDIR /build
COPY apps/server/Cargo.toml apps/server/Cargo.lock ./apps/server/
COPY apps/server/src ./apps/server/src
COPY migrations ./migrations
COPY openapi ./openapi
WORKDIR /build/apps/server
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,target=/build/apps/server/target,sharing=locked \
    cargo build --locked --release && \
    cp target/release/shennong-os-server /tmp/shennong-os-server

FROM debian:bookworm-slim@sha256:7b140f374b289a7c2befc338f42ebe6441b7ea838a042bbd5acbfca6ec875818 AS runtime
RUN apt-get update && \
    apt-get install --no-install-recommends -y ca-certificates curl && \
    rm -rf /var/lib/apt/lists/*
COPY --from=build /tmp/shennong-os-server /usr/local/bin/shennong-os-server
ENV SHENNONG_OS_BIND=0.0.0.0:8080 \
    SHENNONG_OS_COOKIE_SECURE=true \
    SHENNONG_OS_RUN_MIGRATIONS=true
EXPOSE 8080
USER 65532:65532
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
  CMD ["curl", "--fail", "--silent", "--show-error", "http://127.0.0.1:8080/healthz"]
ENTRYPOINT ["/usr/local/bin/shennong-os-server"]
