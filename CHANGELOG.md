# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog 1.1.0], and this project adheres to
[Semantic Versioning 2.0.0].

## [Unreleased]

### Added

- Add first-class chat model and reasoning controls, visible reasoning summaries,
  assistant-ui attachments, `@` mentions, `/` commands, and structured
  clarification choices with a free-form fallback.
- Add a reviewed `run-reproducible-r-analysis` Skill for bounded R and ggplot2
  jobs in the governed Project Runtime.
- Publish the WebUI, control-plane server, and Agent Runtime images to Docker
  Hub from `main`, version tags, and manual GitHub Actions runs.
- Publish one `zerostwo/shennong-os` image containing the WebUI, control-plane
  server, Agent Runtime, gateway, and OS PostgreSQL service.
- Add a three-container Compose deployment that auto-initializes shared service
  credentials and needs only the OS, DB, and Runtime images.
- Add a repository-specific `AGENTS.md` and a local CodeGraph bootstrap rule for
  code-aware maintenance without committing generated index data.
- Add a dedicated `llama-cpp` provider kind for the loopback Bonsai reasoning
  adapter at `host.docker.internal:8081/v1`, including server validation,
  Agent Runtime fetch isolation, OpenAPI, migration, and WebUI settings.
- Add a control-bridge-only systemd socket proxy for the loopback llama.cpp
  reasoning adapter.
- Add administrator-only Resource provider discovery and installation proxies
  so the unified WebUI can install governed ShennongDB datasets without
  exposing the internal service credential.
- Add owner-private personal Agent threads so users can chat without selecting
  a Project, while keeping Project data and tools behind explicit Project RBAC.
- Add persistent user profiles with unique usernames, display names, and
  bounded PNG, JPEG, or WebP avatars.
- Add a production operations center for real control-plane health, users,
  model providers, resource providers, invitations, registration, and audit
  data, with unsupported backup operations labeled explicitly.
- Add a standalone Plugins and Skills workspace backed by the capabilities and
  versioned Skill APIs currently exposed by Shennong.

### Changed

- Move the detailed architecture contract into the standard `docs/` tree,
  publish it fully in English, and document the exact built-in Pi Agent,
  provider, AG-UI, Node.js, and pnpm versions in the README and design record.
- Enable public Shennong DB discovery by default for new personal chats and
  enable both data discovery and reproducible R execution for new Project
  chats, while preserving Project RBAC and Runtime approval requirements.
- Improve assistant message typography, Markdown tables and code blocks, empty
  state actions, composer density, and remove the redundant Agent avatar.
- Default the unified Compose deployment to public Docker Hub application
  images and built-in data/secret paths, reducing the required `.env` surface
  without weakening the Runtime isolation model.
- Replace the seven-image unified deployment default with three public images:
  `zerostwo/shennong-os`, `zerostwo/shennong-db`, and
  `zerostwo/shennong-runtime`.
- Default the retained hardened deployment to public Docker Hub application
  images and built-in data/secret paths, reducing the required `.env` surface
  while preserving its rootless Runtime isolation model.
- Visualize the system, trust boundaries, and request flow in the README and
  document implementation mapping, state ownership, cross-repository contracts,
  and failure semantics in the V1 architecture contract.
- Unify WebUI shell geometry, page gutters, typography, surface colors, active
  navigation, and primary actions around one restrained scientific teal token
  set while preserving the existing information architecture.
- Adapt Agent `db.query_resource` calls to the current ShennongDB typed feature
  and bounded options contract, and pin the local model host alias to the fixed
  control-network gateway instead of Docker's machine-wide host gateway.
- Simplify Projects around a default list view, an Agent-first workspace, and
  direct chat uploads that register private resources before handing durable
  `project://` references to the Agent.
- Move global Docs and search into the sidebar, remove duplicate top-bar
  controls, reorganize Settings, anchor resource drawers, and replace raw JSON
  records with recursive structured values across product surfaces.

### Security

- Prevent administrators from disabling or demoting the final active
  administrator, and revoke a user's active sessions when that account is
  disabled.

## [1.0.0] - 2026-07-18

### Added

- Add the unified Shennong WebUI, migrated from ShennongDB and backed by the
  Shennong OS control plane.
- Add serialized one-time administrator bootstrap, invitation-restricted public
  registration, Argon2id credentials, opaque sessions, CSRF/origin checks,
  rate limits, and Project-scoped `owner/admin/editor/viewer` RBAC.
- Add assistant-ui with native AG-UI history, experimental thread list, and
  interrupt adapters behind a narrow compatibility boundary.
- Add durable threads, messages, Agent runs, events, plans, jobs, artifacts,
  Memory, Skills, providers, audit records, and typed DB/Runtime clients.
- Add Project-authorized AG-UI cursor listing and SSE replay/follow plus the
  assistant-ui history `resume()` adapter, including exactly-once reconnect
  handling that continues the existing Run through its durable terminal event.
- Add durable native assistant-ui approval continuations with transactional
  parent/child Run lineage, immutable tool argument digests, rejection and
  expiry handling, interrupt replay after refresh, and one-use execution grants.
- Add a biomedical Pi Agent harness with governed tools, prompt-injection and
  taint boundaries, EvidenceRef validation, and versioned built-in Skills.
- Add Compute pages for isolated batch jobs, RStudio Server, and JupyterLab.
- Add an on-demand Compute session Open action that mints a short-lived,
  one-time IDE launch ticket without persisting it in WebUI state or history.
- Add unified deployment assets for Shennong OS, headless ShennongDB, and the
  rootless-Docker Shennong Runtime.
- Add a versioned rollback-record template for the exact `1.0.0` repository
  commits and deployed image digests across all three repositories.
- Add fail-closed unified backup, explicit replacement restore, and disposable
  restore-drill assets for OS PostgreSQL, headless DB data, the Runtime SQLite
  journal, deployment secrets/metadata, and explicitly allowlisted workspaces.
- Add idempotent OS-authoritative Project shadow synchronization to the
  ShennongDB `research-projects` contract, including lazy self-healing before
  every Project-scoped data-plane request.
- Add a bounded Project-scoped BioGraph subgraph gateway that applies OS RBAC
  and requires ShennongDB to enforce the same Project boundary.
- Add streamed Project-scoped uploads that register private immutable
  Artifacts and bind their Resource atomically to the active Project.
- Add governed Runtime staging for immutable, Project-bound ShennongDB
  Artifacts through canonical `project://current/resources/.../artifacts/...`
  URIs with bounded download and manifest verification.
- Add least-privilege CI gates for the Rust control plane, production WebUI
  bundle, Agent Runtime, and built-in Skill manifests.
- Add bounded OS-owned virtual project text records and a declarative Pixi
  environment planner as real governed-tool backends.
- Add a control-bridge-only systemd socket proxy and Agent Runtime host-gateway
  mapping for the explicitly supported loopback Ollama provider.
- Add the Apache License 2.0 distribution terms.

### Security

- Treat users, uploads, model output, dependencies, and generated code as
  mutually untrusted; privileged tools are registered and authorized only on
  the server.
- Keep provider, DB, Runtime, bootstrap, invitation, and service credentials
  out of the browser and redact them from logs and persisted Agent events.
- Sign short-lived Runtime capabilities with an OS-only Ed25519 private key;
  Runtime receives only the corresponding verification key.
- Match Project data-plane paths and HTTP methods to the exact headless DB
  allowlist before forwarding them with the deployment service credential.
- Intersect every Agent run with selected Skill tools, project read/write URI
  scopes, and the supported `cpu-small`/`ide-small` Runtime profiles; runs with
  no selected Skill receive only the four core orchestration tools.
- Pin every third-party GitHub Action to a reviewed commit and keep secrets,
  local environments, logs, and build output outside release build contexts.
- Pin CI PostgreSQL plus production Caddy/PostgreSQL images to reviewed
  multi-platform manifest digests.
- Pin the Rust, Debian, and Node base images for every OS service build to
  reviewed multi-platform manifest digests.
- Override Next.js's transitive PostCSS dependency to `8.5.16` so the
  production WebUI is not affected by GHSA-qx2v-qp2m-jg93.
- Keep local Ollama off the LAN by exposing its loopback listener only through
  the exact Shennong control-bridge gateway used by Agent Runtime.
- Scan complete repository history for committed secrets in CI.
- Restrict Agent Resource discovery to the active Project, merging only exact
  Project bindings with the bounded public catalog, and defensively truncate
  anonymous catalog responses behind a short public cache policy.
- Derive upload actor and Project UUIDs from the authenticated OS session,
  discard browser-supplied ownership fields, require CSRF and editor-level
  Project write access, and enforce aligned byte and filename/media-type limits.
- Checksum the complete backup payload, retain a mandatory before-restore
  rollback backup, reject broad or non-canonical restore targets, and access
  Project workspace volumes only through the dedicated rootless endpoint after
  exact Runtime ownership-label and operator-allowlist verification.
- Anchor the independent IDE host to canonical session proxy paths and redact
  launch-ticket query values from both gateway and control-plane request logs.

### Fixed

- Preserve the path-scoped `SameSite=Strict` IDE access cookie while opening an
  IDE from the separate OS origin by redeeming each one-time ticket into a
  no-store, nonce-bound IDE-origin interstitial before same-site navigation.
- Use the published Node 22.23.1 Alpine image tag so the unified WebUI image is
  reproducibly buildable during deployment.
- Remove the unimplemented `environment.ensure` capability and align the
  single-cell Skill with the real `cpu-small` batch profile.
- Replace host-specific example origins and document digest-based deployment
  rollback records plus the complete live V1 acceptance checks.
- Normalize issued and current-session wire formats, reject unsafe login return
  targets, avoid duplicate bootstrap sessions, and remove unimplemented MFA
  claims from the V1 interface.
- Route deeply nested RStudio and JupyterLab assets through the authenticated OS
  session proxy instead of returning the IDE-host fallback 404.
- Prevent duplicate upload submissions and invalidate the Project Resource and
  context-pack caches immediately after a successful Resource registration.
- Accept only archive-internal normalized ClickHouse links during unified
  restore validation and keep backup-path validation variables scope-safe.
- Reconcile the backed-up images, environment, mounts, and gateway configuration
  for exactly the services that were running before an explicit restore.
- Omit absent Provider API keys, reject malformed optional keys defensively, and
  align tool capability digests with the OS 64-hex authorization contract.
- Pause approval-required Pi tool calls without a second provider request, then
  emit the durable native assistant-ui interrupt for later one-use continuation.
- Expose only validated, current-Run backend Evidence IDs to the model after a
  governed tool result so deterministic validation can cite the issued
  EvidenceRef without trusting tool metadata.

[Unreleased]: https://github.com/zerostwo/shennong-os/compare/v1.0.0...HEAD
[1.0.0]: https://github.com/zerostwo/shennong-os/tree/v1.0.0
[Keep a Changelog 1.1.0]: https://keepachangelog.com/en/1.1.0/
[Semantic Versioning 2.0.0]: https://semver.org/spec/v2.0.0.html
