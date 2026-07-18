import { lstat, readFile, readdir } from "node:fs/promises";
import { basename, join, relative, resolve, sep } from "node:path";
import { parse as parseYaml } from "yaml";
import { sha256 } from "./security.js";
import {
  EXECUTABLE_TOOL_NAMES,
  KNOWN_TOOL_NAMES,
  SUPPORTED_COMPUTE_PROFILES,
} from "./tool-registry.js";
import type { ShennongSkillManifest, ValidationFinding } from "./types.js";

const NAME = /^[a-z0-9](?:[a-z0-9-]{0,62}[a-z0-9])?$/;
const ID = /^[a-z0-9][a-z0-9._-]*\/[a-z0-9][a-z0-9-]{0,63}$/;
const SEMVER = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z.-]+)?$/;
const DIGEST = /^sha256:[a-f0-9]{64}$/;
const SAFE_HOST = /^(?:[a-z0-9](?:[a-z0-9.-]*[a-z0-9])?)$/i;
const CONTRACT_NAME = /^[a-z][a-z0-9-]{0,63}$/;
const CONTRACT_TYPE = /^[a-z][a-z0-9.-]{0,63}$/;
const DANGEROUS_SKILL_PATTERNS: Array<[RegExp, string]> = [
  [/ignore (?:all |any )?(?:previous|system|developer) instructions/i, "prompt_override"],
  [/(?:bypass|skip|disable) (?:the )?(?:approval|authorization|permission|policy)/i, "permission_bypass"],
  [/(?:reveal|print|return|exfiltrate).{0,40}(?:token|secret|password|api key)/i, "secret_exfiltration"],
  [/\/var\/run\/docker\.sock|--privileged|host network/i, "container_escape"],
  [/\b(?:sudo|curl|wget)\b|rm\s+-rf/i, "direct_execution"],
  [/https?:\/\//i, "arbitrary_url"],
];

export interface SkillValidationResult {
  directory: string;
  manifest?: ShennongSkillManifest;
  expectedDigest?: string;
  findings: ValidationFinding[];
  valid: boolean;
}

function stable(value: unknown): string {
  if (value === null || typeof value !== "object") return JSON.stringify(value);
  if (Array.isArray(value)) return `[${value.map(stable).join(",")}]`;
  return `{${Object.entries(value as Record<string, unknown>)
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([key, child]) => `${JSON.stringify(key)}:${stable(child)}`)
    .join(",")}}`;
}

function error(findings: ValidationFinding[], code: string, message: string, path?: string): void {
  findings.push({ code, severity: "error", message, ...(path ? { path } : {}) });
}

function warning(findings: ValidationFinding[], code: string, message: string, path?: string): void {
  findings.push({ code, severity: "warning", message, ...(path ? { path } : {}) });
}

function record(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === "object" && !Array.isArray(value);
}

function rejectUnknownKeys(
  value: Record<string, unknown>,
  allowed: readonly string[],
  findings: ValidationFinding[],
  path: string,
): void {
  const unknown = Object.keys(value).filter((key) => !allowed.includes(key));
  if (unknown.length) error(findings, "manifest_unknown_field", `Unknown fields: ${unknown.join(", ")}.`, path);
}

function rejectDuplicates(values: unknown[], findings: ValidationFinding[], path: string): void {
  const strings = values.filter((value): value is string => typeof value === "string");
  if (new Set(strings).size !== strings.length) error(findings, "manifest_duplicate_value", "Array values must be unique.", path);
}

async function filesForDigest(skillDirectory: string, directory = skillDirectory): Promise<Array<[string, string]>> {
  const result: Array<[string, string]> = [];
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    if (entry.name === "shennong.skill.yaml" || entry.name.startsWith(".")) continue;
    const absolute = join(directory, entry.name);
    const status = await lstat(absolute);
    if (status.isSymbolicLink()) throw new Error(`skill_symlink_not_allowed:${relative(skillDirectory, absolute)}`);
    if (entry.isDirectory()) result.push(...(await filesForDigest(skillDirectory, absolute)));
    else if (entry.isFile()) result.push([relative(skillDirectory, absolute).split(sep).join("/"), await readFile(absolute, "utf8")]);
  }
  return result.sort(([left], [right]) => left.localeCompare(right));
}

export async function computeSkillDigest(directory: string, manifest: ShennongSkillManifest): Promise<string> {
  const withoutDigest = structuredClone(manifest);
  withoutDigest.metadata.digest = "";
  const files = await filesForDigest(directory);
  const payload = [stable(withoutDigest), ...files.flatMap(([path, content]) => [path, content])].join("\0");
  return sha256(payload);
}

function parseSkillFrontmatter(content: string): { name?: string; description?: string; unknown: string[] } {
  if (!content.startsWith("---\n")) return { unknown: [] };
  const end = content.indexOf("\n---", 4);
  if (end < 0) return { unknown: [] };
  const value = parseYaml(content.slice(4, end));
  if (!record(value)) return { unknown: [] };
  return {
    ...(typeof value.name === "string" ? { name: value.name } : {}),
    ...(typeof value.description === "string" ? { description: value.description } : {}),
    unknown: Object.keys(value).filter((key) => !["name", "description"].includes(key)),
  };
}

function validateManifestShape(value: unknown, findings: ValidationFinding[]): value is ShennongSkillManifest {
  if (!record(value) || value.apiVersion !== "shennong.one/v1" || value.kind !== "AgentSkill") {
    error(findings, "manifest_identity_invalid", "apiVersion must be shennong.one/v1 and kind must be AgentSkill.");
    return false;
  }
  rejectUnknownKeys(value, ["apiVersion", "kind", "metadata", "spec"], findings, "manifest");
  if (!record(value.metadata) || !record(value.spec)) {
    error(findings, "manifest_shape_invalid", "metadata and spec objects are required.");
    return false;
  }
  const metadata = value.metadata;
  const spec = value.spec;
  rejectUnknownKeys(metadata, ["id", "name", "version", "revision", "digest", "scope", "publisher", "trust"], findings, "metadata");
  rejectUnknownKeys(spec, ["entrypoint", "description", "lifecycle", "compatibility", "contract", "permissions", "validators"], findings, "spec");
  if (!ID.test(String(metadata.id ?? ""))) error(findings, "skill_id_invalid", "metadata.id must be publisher/name.", "metadata.id");
  if (!NAME.test(String(metadata.name ?? ""))) error(findings, "skill_name_invalid", "metadata.name must be lowercase hyphen-case.", "metadata.name");
  if (!SEMVER.test(String(metadata.version ?? ""))) error(findings, "skill_version_invalid", "metadata.version must be SemVer.", "metadata.version");
  if (!Number.isInteger(metadata.revision) || Number(metadata.revision) < 1) error(findings, "skill_revision_invalid", "metadata.revision must be a positive integer.", "metadata.revision");
  if (!DIGEST.test(String(metadata.digest ?? ""))) error(findings, "skill_digest_invalid", "metadata.digest must be sha256:<64 lowercase hex>.", "metadata.digest");
  if (!["platform", "installation", "user", "project"].includes(String(metadata.scope))) error(findings, "skill_scope_invalid", "metadata.scope is invalid.", "metadata.scope");
  if (!["built-in-reviewed", "admin-reviewed", "untrusted"].includes(String(metadata.trust))) error(findings, "skill_trust_invalid", "metadata.trust is invalid.", "metadata.trust");
  if (typeof metadata.publisher !== "string" || !metadata.publisher.trim() || metadata.publisher.length > 128) error(findings, "skill_publisher_invalid", "metadata.publisher must be 1..128 characters.", "metadata.publisher");
  if (typeof spec.entrypoint !== "string" || spec.entrypoint !== "SKILL.md") error(findings, "skill_entrypoint_invalid", "V1 entrypoint must be SKILL.md.", "spec.entrypoint");
  if (typeof spec.description !== "string" || !spec.description.trim() || spec.description.length > 1024) error(findings, "skill_description_invalid", "Description must be 1..1024 characters.", "spec.description");
  if (!["draft", "validating", "active", "deprecated", "disabled", "revoked"].includes(String(spec.lifecycle))) error(findings, "skill_lifecycle_invalid", "Skill lifecycle is invalid.", "spec.lifecycle");
  if (!record(spec.compatibility) || !record(spec.contract) || !record(spec.permissions) || !Array.isArray(spec.validators)) {
    error(findings, "skill_spec_invalid", "compatibility, contract, permissions, and validators are required.", "spec");
    return false;
  }
  const compatibility = spec.compatibility;
  rejectUnknownKeys(compatibility, ["os", "runtime", "dbApi", "pi"], findings, "spec.compatibility");
  for (const key of ["os", "runtime", "dbApi", "pi"]) {
    if (typeof compatibility[key] !== "string" || !compatibility[key]) error(findings, "skill_compatibility_invalid", `Compatibility ${key} is required.`, `spec.compatibility.${key}`);
  }
  const contract = spec.contract;
  rejectUnknownKeys(contract, ["inputs", "outputs"], findings, "spec.contract");
  for (const direction of ["inputs", "outputs"] as const) {
    const fields = contract[direction];
    if (!Array.isArray(fields) || fields.length < 1 || fields.length > 32) {
      error(findings, "skill_contract_invalid", `Contract ${direction} must contain 1..32 fields.`, `spec.contract.${direction}`);
      continue;
    }
    const names = new Set<string>();
    for (const [index, field] of fields.entries()) {
      if (!record(field)) {
        error(findings, "skill_contract_field_invalid", "Contract field must be an object.", `spec.contract.${direction}.${index}`);
        continue;
      }
      rejectUnknownKeys(field, ["name", "type", "required", "description"], findings, `spec.contract.${direction}.${index}`);
      const name = String(field.name ?? "");
      if (!CONTRACT_NAME.test(name)) error(findings, "skill_contract_name_invalid", `Invalid contract field name: ${name}.`, `spec.contract.${direction}.${index}.name`);
      if (names.has(name)) error(findings, "skill_contract_name_duplicate", `Duplicate contract field name: ${name}.`, `spec.contract.${direction}`);
      names.add(name);
      if (!CONTRACT_TYPE.test(String(field.type ?? ""))) error(findings, "skill_contract_type_invalid", `Invalid contract type: ${String(field.type ?? "")}.`, `spec.contract.${direction}.${index}.type`);
      if (typeof field.required !== "boolean") error(findings, "skill_contract_required_invalid", "Contract required must be boolean.", `spec.contract.${direction}.${index}.required`);
      if (typeof field.description !== "string" || !field.description.trim() || field.description.length > 512) error(findings, "skill_contract_description_invalid", "Contract description must be 1..512 characters.", `spec.contract.${direction}.${index}.description`);
    }
  }
  const permissions = spec.permissions;
  rejectUnknownKeys(permissions, ["tools", "projectRead", "projectWrite", "datasetAccess", "networkHosts", "computeProfiles", "approvals"], findings, "spec.permissions");
  for (const key of ["tools", "projectRead", "projectWrite", "networkHosts", "computeProfiles", "approvals"]) {
    if (!Array.isArray(permissions[key]) || permissions[key].some((item) => typeof item !== "string")) {
      error(findings, "skill_permissions_invalid", `Permission ${key} must be a string array.`, `spec.permissions.${key}`);
    } else {
      rejectDuplicates(permissions[key], findings, `spec.permissions.${key}`);
    }
  }
  if (!["public", "private"].includes(String(permissions.datasetAccess))) error(findings, "skill_dataset_access_invalid", "datasetAccess must be public or private.", "spec.permissions.datasetAccess");
  if (spec.validators.some((validator) => typeof validator !== "string")) error(findings, "skill_validators_invalid", "Validators must be a string array.", "spec.validators");
  else rejectDuplicates(spec.validators, findings, "spec.validators");
  return true;
}

export async function validateSkillDirectory(directory: string): Promise<SkillValidationResult> {
  const findings: ValidationFinding[] = [];
  let value: unknown;
  try {
    value = parseYaml(await readFile(join(directory, "shennong.skill.yaml"), "utf8"));
  } catch (reason) {
    error(findings, "manifest_unreadable", reason instanceof Error ? reason.message : "Manifest cannot be read.");
    return { directory, findings, valid: false };
  }
  if (!validateManifestShape(value, findings)) return { directory, findings, valid: false };
  const manifest = value;
  if (basename(directory) !== manifest.metadata.name) error(findings, "skill_directory_mismatch", "Skill directory must match metadata.name.", "metadata.name");

  let skillContent = "";
  try {
    skillContent = await readFile(join(directory, manifest.spec.entrypoint), "utf8");
  } catch {
    error(findings, "skill_entrypoint_missing", "SKILL.md is missing.", "spec.entrypoint");
  }
  if (Buffer.byteLength(skillContent) > 65_536) error(findings, "skill_entrypoint_too_large", "SKILL.md exceeds 64 KiB.", "SKILL.md");
  const frontmatter = parseSkillFrontmatter(skillContent);
  if (frontmatter.name !== manifest.metadata.name) error(findings, "skill_frontmatter_name_mismatch", "SKILL.md name must match manifest metadata.name.", "SKILL.md");
  if (!frontmatter.description) error(findings, "skill_frontmatter_description_missing", "SKILL.md description is required.", "SKILL.md");
  if (frontmatter.unknown.length) warning(findings, "skill_frontmatter_extra", `Unexpected frontmatter: ${frontmatter.unknown.join(", ")}.`, "SKILL.md");
  for (const [pattern, code] of DANGEROUS_SKILL_PATTERNS) {
    if (pattern.test(skillContent)) error(findings, `skill_lint_${code}`, `SKILL.md matched forbidden ${code} guidance.`, "SKILL.md");
  }
  for (const tool of manifest.spec.permissions.tools) {
    if (!KNOWN_TOOL_NAMES.has(tool)) error(findings, "skill_tool_unknown", `Unknown governed tool: ${tool}.`, "spec.permissions.tools");
    else if (manifest.spec.lifecycle === "active" && !EXECUTABLE_TOOL_NAMES.has(tool)) {
      error(
        findings,
        "skill_tool_unavailable",
        `Active Skill tool has no V1 execution backend: ${tool}.`,
        "spec.permissions.tools",
      );
    }
  }
  for (const validator of manifest.spec.validators) {
    if (!manifest.spec.permissions.tools.includes(validator)) {
      error(
        findings,
        "skill_validator_not_permitted",
        `Validator must also be declared as a governed tool: ${validator}.`,
        "spec.validators",
      );
    } else if (!EXECUTABLE_TOOL_NAMES.has(validator)) {
      error(
        findings,
        "skill_validator_unavailable",
        `Validator has no V1 execution backend: ${validator}.`,
        "spec.validators",
      );
    }
  }
  for (const profile of manifest.spec.permissions.computeProfiles) {
    if (!SUPPORTED_COMPUTE_PROFILES.has(profile)) {
      error(
        findings,
        "skill_compute_profile_unavailable",
        `Unsupported V1 Runtime profile: ${profile}.`,
        "spec.permissions.computeProfiles",
      );
    }
  }
  if (
    manifest.spec.permissions.tools.includes("runtime.submit_job")
    && !manifest.spec.permissions.computeProfiles.includes("cpu-small")
  ) {
    error(
      findings,
      "skill_batch_profile_missing",
      "runtime.submit_job requires the cpu-small V1 compute profile.",
      "spec.permissions.computeProfiles",
    );
  }
  if (
    manifest.spec.permissions.tools.some((tool) =>
      ["project.list_files", "project.read_file"].includes(tool)
    )
    && manifest.spec.permissions.projectRead.length === 0
  ) {
    error(
      findings,
      "skill_project_read_scope_missing",
      "Project read tools require at least one projectRead scope.",
      "spec.permissions.projectRead",
    );
  }
  if (
    manifest.spec.permissions.tools.includes("project.write_file")
    && manifest.spec.permissions.projectWrite.length === 0
  ) {
    error(
      findings,
      "skill_project_write_scope_missing",
      "project.write_file requires at least one projectWrite scope.",
      "spec.permissions.projectWrite",
    );
  }
  for (const host of manifest.spec.permissions.networkHosts) {
    if (host === "*" || !SAFE_HOST.test(host)) error(findings, "skill_network_host_invalid", `Invalid network host: ${host}.`, "spec.permissions.networkHosts");
  }
  for (const path of [...manifest.spec.permissions.projectRead, ...manifest.spec.permissions.projectWrite]) {
    if (!path.startsWith("project://current/") || path.includes("..")) error(findings, "skill_project_path_invalid", `Project permission is not scoped: ${path}.`, "spec.permissions");
  }
  if (manifest.metadata.trust === "untrusted") {
    try {
      const files = await filesForDigest(directory);
      if (files.some(([path]) => path.startsWith("scripts/"))) error(findings, "untrusted_skill_script", "Untrusted V1 Skills cannot bundle executable scripts.", "scripts");
    } catch (reason) {
      error(findings, "skill_bundle_invalid", reason instanceof Error ? reason.message : "Skill bundle is invalid.");
    }
  }
  let expectedDigest: string | undefined;
  try {
    expectedDigest = await computeSkillDigest(directory, manifest);
    if (manifest.metadata.digest !== expectedDigest) error(findings, "skill_digest_mismatch", `Expected ${expectedDigest}.`, "metadata.digest");
  } catch (reason) {
    error(findings, "skill_digest_failed", reason instanceof Error ? reason.message : "Digest calculation failed.");
  }
  return {
    directory: resolve(directory),
    manifest,
    ...(expectedDigest ? { expectedDigest } : {}),
    findings,
    valid: !findings.some(({ severity }) => severity === "error"),
  };
}

export async function validateSkillsRoot(root: string): Promise<SkillValidationResult[]> {
  const results: SkillValidationResult[] = [];
  for (const entry of await readdir(root, { withFileTypes: true })) {
    if (!entry.isDirectory() || entry.name.startsWith(".")) continue;
    results.push(await validateSkillDirectory(join(root, entry.name)));
  }
  return results.sort((left, right) => left.directory.localeCompare(right.directory));
}
