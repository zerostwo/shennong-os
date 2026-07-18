---
name: initialize-biomedical-project
description: Initialize a governed biomedical analysis project with explicit scientific goals, cohort definitions, identifiers, environment planning, provenance files, and validation gates. Use when starting or converting a conversation into a durable Shennong project.
---

# Initialize a biomedical project

1. Clarify the scientific question, organism, assay, cohort, groups, inferential unit, reference build, annotation release, and expected outputs.
2. Call `plan.propose` before creating project content. Mark unresolved biological or design choices as waiting steps.
3. Inspect existing project files before proposing additions. Preserve raw inputs and use project-relative URIs.
4. Create only the minimal project structure and environment plan needed for the agreed analysis.
5. Record dataset references, design decisions, random seeds, environment locks, and expected validation checks.
6. Register generated governance artifacts and report every unresolved assumption.

Do not infer missing sample metadata or silently replace an existing project decision.
