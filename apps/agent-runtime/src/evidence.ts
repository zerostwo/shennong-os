import type { EvidenceRef, JsonValue, ValidationFinding } from "./types.js";

const EVIDENCE_ID = /^[a-zA-Z0-9][a-zA-Z0-9._:-]{0,127}$/;
const SHA256 = /^sha256:[a-f0-9]{64}$/;

export function isEvidenceRef(value: unknown): value is EvidenceRef {
  if (!value || typeof value !== "object") return false;
  const candidate = value as Partial<EvidenceRef>;
  return (
    typeof candidate.id === "string" &&
    EVIDENCE_ID.test(candidate.id) &&
    typeof candidate.runId === "string" &&
    typeof candidate.sourceId === "string" &&
    ["dataset", "query", "tool-result", "artifact", "publication"].includes(candidate.kind ?? "") &&
    (candidate.digest === undefined || SHA256.test(candidate.digest))
  );
}

export function extractEvidence(value: JsonValue, expectedRunId: string): EvidenceRef[] {
  if (!value || typeof value !== "object" || Array.isArray(value)) return [];
  const candidate = value as Record<string, JsonValue>;
  const rows = candidate.evidence;
  if (!Array.isArray(rows)) return [];
  const byId = new Map<string, EvidenceRef>();
  for (const row of rows) {
    if (isEvidenceRef(row) && row.runId === expectedRunId) byId.set(row.id, row);
  }
  return [...byId.values()];
}

export function validateCitationIds(citationIds: string[] | undefined, evidence: EvidenceRef[]): ValidationFinding[] {
  const known = new Set(evidence.map(({ id }) => id));
  const findings: ValidationFinding[] = [];
  for (const id of citationIds ?? []) {
    if (!known.has(id)) {
      findings.push({
        code: "citation_unknown",
        severity: "error",
        message: `Citation ${id} is not backed by an EvidenceRef from this run.`,
        path: "result.citationIds",
      });
    }
  }
  return findings;
}
