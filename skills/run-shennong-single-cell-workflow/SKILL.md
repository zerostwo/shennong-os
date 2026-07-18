---
name: run-shennong-single-cell-workflow
description: Plan and run a governed Shennong single-cell or single-nucleus RNA-seq workflow with sample-level design, quality control, versioned environments, isolated Runtime jobs, provenance, artifacts, and deterministic validation. Use after input readiness is established.
---

# Run a Shennong single-cell workflow

1. Require an active project, inspected inputs, organism, reference, sample metadata, groups, patients, and biological replicates.
2. Call `plan.propose` with separate input audit, environment, QC, analysis, artifact, and validation steps.
3. Use `environment.plan` to produce a reviewable declarative plan. Resolve and materialize its lock only inside a reviewed `cpu-small` Runtime job.
4. Write reviewable scripts and configuration under project-relative paths. Keep raw data unchanged.
5. Submit bounded Runtime jobs with explicit inputs, outputs, resources, timeout, seed, and environment lock.
6. Evaluate cell-level QC descriptively but perform biological inference at the sample or patient level.
7. Register figures, tables, objects, scripts, and reports with complete provenance.
8. Run `analysis.validate`; stop scientific interpretation when it fails.

Do not claim successful computation until the Runtime job and expected artifacts are verified.
