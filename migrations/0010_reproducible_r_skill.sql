INSERT INTO skills (id, owner_user_id, slug, name, description, trust_level, lifecycle, manifest, current_version)
VALUES (
  '93fd2a56-45db-4c55-9689-477584ef9b77'::uuid,
  NULL,
  'run-reproducible-r-analysis',
  'run-reproducible-r-analysis',
  'Execute bounded R analyses and ggplot2 visualizations through governed Project and Runtime tools.',
  'builtin_signed',
  'active',
  $manifest${"apiVersion":"shennong.one/v1","kind":"AgentSkill","metadata":{"id":"zerostwo/run-reproducible-r-analysis","name":"run-reproducible-r-analysis","version":"1.0.0","revision":1,"digest":"sha256:35f603bb083f829b5350565537190e2b5ae8e181bbdba6c55c4fe4b0e71b088c","scope":"platform","publisher":"zerostwo","trust":"built-in-reviewed"},"spec":{"entrypoint":"SKILL.md","description":"Execute bounded R analyses and ggplot2 visualizations through governed Project and Runtime tools.","lifecycle":"active","compatibility":{"os":">=1.0.0 <2.0.0","runtime":">=1.0.0 <2.0.0","dbApi":"v1","pi":"0.80.10"},"contract":{"inputs":[{"name":"project-ref","type":"project-ref","required":true,"description":"Authorized Shennong OS Project for scripts, Runtime jobs, artifacts, and approvals."},{"name":"analysis-request","type":"analysis-design","required":true,"description":"R computation or visualization request, inputs, expected outputs, and verification criteria."}],"outputs":[{"name":"runtime-job","type":"runtime-job","required":true,"description":"Terminal governed Runtime job with bounded logs and environment metadata."},{"name":"artifact-bundle","type":"result-bundle","required":true,"description":"Registered scripts, figures, and tables with immutable provenance."}]},"permissions":{"tools":["plan.propose","plan.update","db.inspect_resource","db.query_resource","db.get_provenance","project.list_files","project.read_file","project.write_file","environment.plan","runtime.submit_job","runtime.get_job","runtime.cancel_job","artifact.register","analysis.validate"],"projectRead":["project://current/"],"projectWrite":["project://current/scripts/","project://current/results/","project://current/reports/","project://current/logs/","project://current/environments/","project://current/.shennong/"],"datasetAccess":"private","networkHosts":[],"computeProfiles":["cpu-small"],"approvals":["runtime.compute","project.write","runtime.cancel","artifact.register"]},"validators":["analysis.validate"]}}$manifest$::jsonb,
  1
);

INSERT INTO skill_versions (skill_id, version, content, content_sha256, package_version, change_note, created_by_user_id)
VALUES (
  '93fd2a56-45db-4c55-9689-477584ef9b77'::uuid,
  1,
  $skill$---
name: run-reproducible-r-analysis
description: Plan and run bounded R analyses and ggplot2 visualizations in the governed Shennong Runtime, including R built-in datasets, project inputs, scripts, artifacts, provenance, and validation. Use when a user asks to execute R code, create a plot, or compute a reproducible statistical result.
---

# Run a reproducible R analysis

1. Require an active Project before executing code. If the conversation is personal, explain that Runtime work needs a Project and help the user choose one.
2. Call `plan.propose` with the smallest useful analysis, artifact, and verification steps.
3. For an R built-in dataset, do not claim that an upload is required. Use the dataset by name from the reviewed R script.
4. Write a reviewable R script under `project://current/scripts/` and write outputs under `project://current/results/`.
5. Submit a bounded `cpu-small` Runtime job with explicit argv, inputs, outputs, timeout, seed when relevant, and the R environment requirements.
6. Read the terminal job status and logs. Do not claim success until expected output artifacts exist and are non-empty.
7. Register durable plots, tables, and scripts with their producing job, environment, command, and content digests.
8. Run `analysis.validate` when the result contains biomedical or inferential claims. For purely demonstrative built-in data, clearly label the output as a software demonstration.

Never execute host paths or arbitrary mounts, and never bypass Runtime approval.
$skill$,
  '8279546c553774f144e585d60656933acb197e2d884b7582de17d8e3822efd98',
  '1.0.0',
  'Built-in governed R and ggplot2 Runtime Skill.',
  NULL
);
