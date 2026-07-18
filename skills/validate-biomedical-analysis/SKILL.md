---
name: validate-biomedical-analysis
description: Apply deterministic quality gates to a biomedical analysis, covering organism and reference compatibility, identifiers, sample uniqueness, biological replication, inferential unit, contrast, pairing, multiple testing, artifacts, provenance, and EvidenceRef citations. Use before marking work complete.
---

# Validate a biomedical analysis

1. Retrieve the producing dataset provenance, project design, Runtime job, and registered artifacts.
2. Build a validation input from observed metadata rather than narrative claims.
3. Check organism, reference build, annotation release, sample uniqueness, groups, replicates, patients, inference level, contrast, pairing, result rows, effect sizes, adjusted values, and expected artifacts.
4. Call `analysis.validate` with all EvidenceRef records used by the result.
5. Treat `fail` as blocking. Treat `warn` as a required limitation in downstream reporting.
6. Keep deterministic findings unchanged when summarizing them for the user.

Use these V1 gates:

- Fewer than two biological replicates in any contrasted group is `fail`; two is `warn`. Thousands of cells from one subject are still one biological replicate.
- A between-condition test with `inferentialUnit=cell` is pseudoreplication and `fail`. Pseudobulk changes the measurement unit, but one subject per condition still cannot support population-level inference.
- One subject in every condition means condition and subject identity are completely confounded and `fail`.
- Paired or repeated-measures analysis must identify a real pairing key and use subject/patient as the inferential unit.
- Required organism, reference build, annotation release, sample/patient counts, group replicate counts, contrast, result row count, effect-size presence, multiple-testing requirement, expected artifacts, and EvidenceRef records must come from observed metadata. Missing required metadata is blocking; never guess it.
- Expected artifacts must exist, be non-empty, and have content digests. Every cited ID must resolve to an EvidenceRef issued for this run.
- With a failed design, permit only clearly labelled exploratory description of the observed subjects. Do not use wording that implies a reliable, generalizable, or causal condition effect.

Do not replace a failed check with a model judgment or infer evidence that was not returned by a governed tool.
