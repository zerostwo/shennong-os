---
name: interpret-biomedical-results
description: Interpret validated Shennong biomedical outputs into evidence-linked findings, limitations, figure or table descriptions, and next-step hypotheses while separating descriptive, inferential, and biological claims. Use only after result provenance and validation are available.
---

# Interpret biomedical results

1. Read registered result artifacts, producing job metadata, dataset provenance, and the latest deterministic validation report.
2. Stop if validation failed. Carry every warning into the interpretation.
3. Report cohort and group sizes, inferential unit, effect direction and magnitude, uncertainty, adjusted values, and relevant preprocessing.
4. Separate direct observations, supported statistical inferences, and biological hypotheses.
5. Attach an EvidenceRef identifier to each stored-data claim and preserve Resource and artifact versions.
6. Describe alternative explanations, confounding, missing annotations, and analyses not supported by the available data.
7. Propose next steps as hypotheses or validation work, not as completed findings.

Never turn correlation into causation or generalize beyond the analyzed cohort.
