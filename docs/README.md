# Shennong OS documentation

This directory contains the maintained design documentation for Shennong OS.

- [Architecture and acceptance contract](architecture.md): component
  boundaries, trust zones, request flows, state ownership, deployment profiles,
  failure semantics, and release gates.
- [Deployment guide](../deploy/README.md): hardened rootless production setup.
- [Simple deployment](../deploy/SIMPLE.md): trusted single-user three-image
  installation.
- [OpenAPI contract](../openapi/os-api.yaml): normative browser and service API.

Keep the root [README](../README.md) concise and visual. Detailed design changes
belong here and must be updated with the implementation and `CHANGELOG.md` in
the same change.
