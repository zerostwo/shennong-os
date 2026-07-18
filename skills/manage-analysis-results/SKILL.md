---
name: manage-analysis-results
description: Discover, verify, register, and reuse versioned Shennong analysis outputs with immutable provenance, content hashes, environment and prompt versions, and explicit validation status. Use when collecting Runtime outputs or preparing results for later interpretation.
---

# Manage analysis results

1. Read the producing job status and confirm its successful exit before accepting outputs.
2. Inspect expected result paths and reject missing, empty, or unrecognized files.
3. Preserve the producing dataset references, query digests, runtime image, environment locks, command, seed, Prompt versions, and Skill versions.
4. Run `analysis.validate` before registration.
5. Register each durable artifact with a stable type and content digest.
6. Update project result indexes without replacing raw outputs or prior provenance records.
7. Report validation warnings and failed artifacts explicitly.

Treat a filename or report narrative as insufficient evidence of successful analysis.
