import { validateCitationIds } from "./evidence.js";
import type { AnalysisValidationInput, AnalysisValidationReport, ValidationFinding } from "./types.js";

export const ANALYSIS_VALIDATOR_VERSION = "shennong-analysis-validator/v1";

function normalized(value: string | undefined): string | undefined {
  return value?.trim().toLowerCase().replaceAll(/[_\s-]+/g, "");
}

function mismatch(
  findings: ValidationFinding[],
  code: string,
  label: string,
  expected: string | undefined,
  actual: string | undefined,
  path: string,
): void {
  if (expected && !actual) {
    findings.push({
      code: code.replace(/_mismatch$/, "_missing"),
      severity: "error",
      message: `${label} is required by the design but absent from dataset provenance.`,
      path,
    });
    return;
  }
  if (expected && actual && normalized(expected) !== normalized(actual)) {
    findings.push({
      code,
      severity: "error",
      message: `${label} mismatch: expected ${expected}, received ${actual}.`,
      path,
    });
  }
}

export function validateAnalysis(input: AnalysisValidationInput, now = new Date()): AnalysisValidationReport {
  const findings: ValidationFinding[] = [];
  mismatch(findings, "organism_mismatch", "Organism", input.expected?.organism, input.dataset.organism, "dataset.organism");
  mismatch(
    findings,
    "reference_build_mismatch",
    "Reference build",
    input.expected?.referenceBuild,
    input.dataset.referenceBuild,
    "dataset.referenceBuild",
  );
  mismatch(
    findings,
    "annotation_release_mismatch",
    "Annotation release",
    input.expected?.annotationRelease,
    input.dataset.annotationRelease,
    "dataset.annotationRelease",
  );

  if (!Number.isInteger(input.dataset.sampleCount) || input.dataset.sampleCount <= 0) {
    findings.push({ code: "sample_count_invalid", severity: "error", message: "Sample count must be positive.", path: "dataset.sampleCount" });
  }
  if (
    input.dataset.uniqueSampleCount !== undefined &&
    input.dataset.uniqueSampleCount !== input.dataset.sampleCount
  ) {
    findings.push({ code: "duplicate_sample_ids", severity: "error", message: "Sample identifiers are not unique.", path: "dataset.uniqueSampleCount" });
  }
  for (const [group, count] of Object.entries(input.dataset.groupReplicates ?? {})) {
    if (count < 2) {
      findings.push({
        code: "biological_replicates_missing",
        severity: "error",
        message: `Group ${group} has fewer than two biological replicates.`,
        path: `dataset.groupReplicates.${group}`,
      });
    } else if (count < 3) {
      findings.push({
        code: "biological_replicates_low",
        severity: "warning",
        message: `Group ${group} has only ${count} biological replicates; inference is fragile.`,
        path: `dataset.groupReplicates.${group}`,
      });
    }
  }
  const replicateCounts = Object.values(input.dataset.groupReplicates ?? {});
  if (replicateCounts.length >= 2 && replicateCounts.every((count) => count === 1)) {
    findings.push({
      code: "condition_subject_confounding",
      severity: "error",
      message: "Every condition has only one biological subject, so condition and subject identity are completely confounded.",
      path: "dataset.groupReplicates",
    });
  }
  if (input.dataset.inferentialUnit === "cell" && input.design?.contrast) {
    findings.push({
      code: "cell_pseudoreplication",
      severity: "error",
      message: "Cells cannot be treated as independent biological replicates for a between-condition contrast.",
      path: "dataset.inferentialUnit",
    });
  }
  if (
    input.dataset.patientCount !== undefined &&
    input.dataset.patientCount < input.dataset.sampleCount &&
    input.dataset.inferentialUnit !== "patient"
  ) {
    findings.push({
      code: "inferential_unit_invalid",
      severity: "error",
      message: "Repeated samples per patient require patient-level inference or an explicit repeated-measures model.",
      path: "dataset.inferentialUnit",
    });
  }

  const groups = new Set(input.design?.groups ?? []);
  const contrast = input.design?.contrast;
  if (contrast && (!groups.has(contrast[0]) || !groups.has(contrast[1]))) {
    findings.push({ code: "contrast_unknown_group", severity: "error", message: "Contrast references a group absent from the design.", path: "design.contrast" });
  }
  if (input.design?.paired && !input.design.pairingKeyPresent) {
    findings.push({ code: "pairing_key_missing", severity: "error", message: "Paired analysis requires a pairing key.", path: "design.pairingKeyPresent" });
  }
  if (input.result.rowCount <= 0) {
    findings.push({ code: "result_empty", severity: "error", message: "The analysis returned no rows.", path: "result.rowCount" });
  }
  if (!input.result.effectSizePresent) {
    findings.push({ code: "effect_size_missing", severity: "warning", message: "Result does not report an effect size.", path: "result.effectSizePresent" });
  }
  if (input.design?.multipleTestingRequired && !input.result.adjustedPValuePresent) {
    findings.push({ code: "multiple_testing_missing", severity: "error", message: "Multiple testing was required but adjusted p-values are absent.", path: "result.adjustedPValuePresent" });
  }
  if (input.result.expectedArtifactCount !== undefined) {
    const artifacts = input.result.artifacts ?? [];
    if (artifacts.length < input.result.expectedArtifactCount) {
      findings.push({ code: "artifact_missing", severity: "error", message: "One or more expected artifacts are absent.", path: "result.artifacts" });
    }
    for (const artifact of artifacts) {
      if (artifact.sizeBytes <= 0 || !artifact.digest) {
        findings.push({ code: "artifact_invalid", severity: "error", message: `Artifact ${artifact.id} is empty or lacks a digest.`, path: "result.artifacts" });
      }
    }
  }
  findings.push(...validateCitationIds(input.result.citationIds, input.evidence));
  const status = findings.some(({ severity }) => severity === "error")
    ? "fail"
    : findings.some(({ severity }) => severity === "warning")
      ? "warn"
      : "pass";
  return { status, findings, checkedAt: now.toISOString(), validatorVersion: ANALYSIS_VALIDATOR_VERSION };
}
