---
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
