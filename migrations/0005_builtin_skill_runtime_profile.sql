-- Replace the pre-release single-cell Skill revision with the executable V1 contract.
-- Historical version 1 remains append-only, while active thread pins move to revision 2.
INSERT INTO skill_versions (
  skill_id, version, content, content_sha256, package_version, change_note, created_by_user_id
)
VALUES (
  '7d51e897-0b5f-5a8e-bc1a-57e561fc42c1'::uuid,
  2,
  $skill$---
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
$skill$,
  '1bfa7565a498cbe679fed5f1a36dfae2c8fa7a16d17d3a179f59f88dd6e6f310',
  '1.0.0',
  'Align executable tools and Runtime profiles with the V1 governed backends.',
  NULL
);

UPDATE skills
SET manifest = $manifest${"apiVersion":"shennong.one/v1","kind":"AgentSkill","metadata":{"id":"zerostwo/run-shennong-single-cell-workflow","name":"run-shennong-single-cell-workflow","version":"1.0.0","revision":2,"digest":"sha256:7fa423acfa97a0ee4bfdda957be39e072bbeffdb34d5cf8a8810e5284944d622","scope":"platform","publisher":"zerostwo","trust":"built-in-reviewed"},"spec":{"entrypoint":"SKILL.md","description":"Execute a reproducible Shennong single-cell workflow through governed project and Runtime tools.","lifecycle":"active","compatibility":{"os":">=1.0.0 <2.0.0","runtime":">=1.0.0 <2.0.0","dbApi":"v1","pi":"0.80.10"},"contract":{"inputs":[{"name":"project-ref","type":"project-ref","required":true,"description":"Authorized Shennong OS project scope for all reads, writes, approvals, and Runtime jobs."},{"name":"dataset-ref","type":"dataset-ref","required":true,"description":"Inspected single-cell or single-nucleus input with immutable provenance."},{"name":"analysis-design","type":"analysis-design","required":true,"description":"Organism, assay, groups, contrasts, pairing, inferential unit, methods, and expected artifacts."}],"outputs":[{"name":"runtime-jobs","type":"runtime-job-list","required":true,"description":"Governed JobSpecs and terminal job records for every executed step."},{"name":"artifact-bundle","type":"result-bundle","required":true,"description":"Registered tables, figures, objects, logs, and complete reproducibility metadata."},{"name":"validation-report","type":"validation-report","required":true,"description":"Deterministic post-run validation that blocks completion on any failure."}]},"permissions":{"tools":["plan.propose","plan.update","db.inspect_resource","db.query_resource","db.get_provenance","project.list_files","project.read_file","project.write_file","environment.plan","runtime.submit_job","runtime.get_job","runtime.cancel_job","artifact.register","analysis.validate"],"projectRead":["project://current/"],"projectWrite":["project://current/scripts/","project://current/results/","project://current/reports/","project://current/logs/","project://current/environments/","project://current/.shennong/"],"datasetAccess":"private","networkHosts":[],"computeProfiles":["cpu-small"],"approvals":["runtime.compute","project.write","runtime.cancel","artifact.register"]},"validators":["analysis.validate"]}}$manifest$::jsonb,
    current_version = 2,
    updated_at = NOW()
WHERE id = '7d51e897-0b5f-5a8e-bc1a-57e561fc42c1'::uuid
  AND trust_level = 'builtin_signed';

UPDATE thread_skills
SET skill_version = 2
WHERE skill_id = '7d51e897-0b5f-5a8e-bc1a-57e561fc42c1'::uuid
  AND skill_version = 1;
