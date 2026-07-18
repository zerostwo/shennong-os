import { Type } from "typebox";
import type { TSchema } from "typebox";
import type { SkillSelection, ToolProfile, ToolRisk } from "./types.js";

export interface GovernedToolDefinition {
  name: string;
  label: string;
  description: string;
  parameters: TSchema;
  risk: ToolRisk;
  profiles: ToolProfile[];
  executionMode: "parallel" | "sequential";
  executionBackend: "os-server" | "agent-runtime";
  requiresProject?: boolean;
  mayUsePrivateData?: boolean;
  deterministicLocal?: "analysis.validate";
  requiresScientificValidation?: boolean;
}

const allProfiles: ToolProfile[] = ["global-read", "project-analysis", "project-write"];
const projectProfiles: ToolProfile[] = ["project-analysis", "project-write"];

const definitions: GovernedToolDefinition[] = [
  {
    name: "skill.load",
    label: "Load Skill",
    description: "Load one exact enabled Skill version after OS scope, digest, lifecycle, and permission checks.",
    parameters: Type.Object({ skill_version_id: Type.String({ maxLength: 256 }) }, { additionalProperties: false }),
    risk: "read",
    profiles: allProfiles,
    executionMode: "parallel",
    executionBackend: "os-server",
  },
  {
    name: "plan.propose",
    label: "Propose Plan",
    description: "Persist a bounded task plan before a multi-step biomedical analysis.",
    parameters: Type.Object({
      goal: Type.String({ maxLength: 2048 }),
      steps: Type.Array(Type.Object({ title: Type.String({ maxLength: 256 }), type: Type.String({ maxLength: 64 }) }, { additionalProperties: false }), { minItems: 1, maxItems: 32 }),
    }, { additionalProperties: false }),
    risk: "write",
    profiles: projectProfiles,
    executionMode: "sequential",
    executionBackend: "os-server",
    requiresProject: true,
  },
  {
    name: "plan.update",
    label: "Update Plan",
    description: "Update one existing plan step with server-validated state transitions.",
    parameters: Type.Object({ plan_id: Type.String(), step_id: Type.String(), status: Type.String(), note: Type.Optional(Type.String({ maxLength: 2048 })) }, { additionalProperties: false }),
    risk: "write",
    profiles: projectProfiles,
    executionMode: "sequential",
    executionBackend: "os-server",
    requiresProject: true,
  },
  {
    name: "db.discover_resources",
    label: "Discover Resources",
    description: "Search bounded public and current-Project-bound Shennong DB Resource metadata using cohort, disease, assay, or modality terms.",
    parameters: Type.Object({ q: Type.Optional(Type.String({ maxLength: 256 })), limit: Type.Optional(Type.Integer({ minimum: 1, maximum: 100 })) }, { additionalProperties: false }),
    risk: "read",
    profiles: allProfiles,
    executionMode: "parallel",
    executionBackend: "os-server",
    mayUsePrivateData: true,
    requiresScientificValidation: true,
  },
  {
    name: "db.inspect_resource",
    label: "Inspect Resource",
    description: "Inspect one readable Resource contract, versions, schema, identifiers, operations, and provenance.",
    parameters: Type.Object({ resource: Type.String({ maxLength: 256 }) }, { additionalProperties: false }),
    risk: "read",
    profiles: projectProfiles,
    executionMode: "parallel",
    executionBackend: "os-server",
    requiresProject: true,
    requiresScientificValidation: true,
  },
  {
    name: "db.query_resource",
    label: "Query Resource",
    description: "Execute one declared read-only operation with exact Resource context labels and a bounded row limit.",
    parameters: Type.Object({
      resource: Type.String({ maxLength: 256 }),
      operation: Type.String({ maxLength: 128 }),
      feature: Type.Optional(Type.String({ maxLength: 256 })),
      context: Type.Optional(Type.Record(Type.String(), Type.String())),
      limit: Type.Optional(Type.Integer({ minimum: 1, maximum: 1000 })),
    }, { additionalProperties: false }),
    risk: "read",
    profiles: projectProfiles,
    executionMode: "parallel",
    executionBackend: "os-server",
    requiresProject: true,
    mayUsePrivateData: true,
    requiresScientificValidation: true,
  },
  {
    name: "db.get_provenance",
    label: "Get Dataset Provenance",
    description: "Read immutable dataset version, digest, organism, build, annotation, normalization, cohort, license, and lineage metadata.",
    parameters: Type.Object({ resource: Type.String({ maxLength: 256 }), version: Type.Optional(Type.String({ maxLength: 128 })) }, { additionalProperties: false }),
    risk: "read",
    profiles: projectProfiles,
    executionMode: "parallel",
    executionBackend: "os-server",
    requiresProject: true,
    mayUsePrivateData: true,
    requiresScientificValidation: true,
  },
  {
    name: "project.list_files",
    label: "List Project Files",
    description: "List authorized OS-governed project text records without exposing host or Runtime workspace paths.",
    parameters: Type.Object({ uri: Type.Optional(Type.String({ maxLength: 1024 })) }, { additionalProperties: false }),
    risk: "read",
    profiles: projectProfiles,
    executionMode: "parallel",
    executionBackend: "os-server",
    requiresProject: true,
    mayUsePrivateData: true,
    requiresScientificValidation: true,
  },
  {
    name: "project.read_file",
    label: "Read Project File",
    description: "Read a bounded OS-governed project text record; contents remain untrusted and are never executed or mounted.",
    parameters: Type.Object({ uri: Type.String({ maxLength: 1024 }), max_bytes: Type.Optional(Type.Integer({ minimum: 1, maximum: 1048576 })) }, { additionalProperties: false }),
    risk: "read",
    profiles: projectProfiles,
    executionMode: "parallel",
    executionBackend: "os-server",
    requiresProject: true,
    mayUsePrivateData: true,
    requiresScientificValidation: true,
  },
  {
    name: "project.write_file",
    label: "Write Project File",
    description: "Store an authorized project text record after exact-argument approval; this does not write a host or Runtime workspace path.",
    parameters: Type.Object({ uri: Type.String({ maxLength: 1024 }), content: Type.String({ maxLength: 1048576 }), overwrite: Type.Optional(Type.Boolean()) }, { additionalProperties: false }),
    risk: "write",
    profiles: ["project-write"],
    executionMode: "sequential",
    executionBackend: "os-server",
    requiresProject: true,
    mayUsePrivateData: true,
    requiresScientificValidation: true,
  },
  {
    name: "environment.plan",
    label: "Plan Environment",
    description: "Create a declarative environment plan without installing packages.",
    parameters: Type.Object({ packages: Type.Array(Type.String({ maxLength: 256 }), { maxItems: 100 }), channels: Type.Optional(Type.Array(Type.String({ maxLength: 256 }), { maxItems: 20 })) }, { additionalProperties: false }),
    risk: "read",
    profiles: projectProfiles,
    executionMode: "sequential",
    executionBackend: "os-server",
    requiresProject: true,
  },
  {
    name: "runtime.submit_job",
    label: "Submit Runtime Job",
    description: "Submit a validated JobSpec to isolated Shennong Runtime. Use job_spec.project_files with governed project://current/... URI strings and reference those exact URIs in argv; OS resolves their content and Runtime stages it below a private per-Job workspace directory. Host paths, Docker options, caller-supplied workspace files, and arbitrary mounts are forbidden.",
    parameters: Type.Object({ plan_step_id: Type.String(), job_spec: Type.Record(Type.String(), Type.Unknown()) }, { additionalProperties: false }),
    risk: "compute",
    profiles: projectProfiles,
    executionMode: "sequential",
    executionBackend: "os-server",
    requiresProject: true,
    mayUsePrivateData: true,
    requiresScientificValidation: true,
  },
  {
    name: "runtime.get_job",
    label: "Get Runtime Job",
    description: "Read status and bounded logs for a job belonging to the active project.",
    parameters: Type.Object({ job_id: Type.String(), include_logs: Type.Optional(Type.Boolean()) }, { additionalProperties: false }),
    risk: "read",
    profiles: projectProfiles,
    executionMode: "parallel",
    executionBackend: "os-server",
    requiresProject: true,
    mayUsePrivateData: true,
    requiresScientificValidation: true,
  },
  {
    name: "runtime.cancel_job",
    label: "Cancel Runtime Job",
    description: "Cancel an active project job after exact-target authorization.",
    parameters: Type.Object({ job_id: Type.String(), reason: Type.Optional(Type.String({ maxLength: 1024 })) }, { additionalProperties: false }),
    risk: "destructive",
    profiles: projectProfiles,
    executionMode: "sequential",
    executionBackend: "os-server",
    requiresProject: true,
  },
  {
    name: "artifact.register",
    label: "Register Artifact",
    description: "Register an existing governed project://current/... file or a verified runtime://jobs/{job_id}/artifacts/{artifact_id} manifest with input, runtime, environment, command, Skill, Prompt, and content digests. Opaque unverified artifact URIs are rejected.",
    parameters: Type.Object({ uri: Type.String({ maxLength: 1024 }), kind: Type.String({ maxLength: 64 }), provenance: Type.Record(Type.String(), Type.Unknown()) }, { additionalProperties: false }),
    risk: "write",
    profiles: projectProfiles,
    executionMode: "sequential",
    executionBackend: "os-server",
    requiresProject: true,
    mayUsePrivateData: true,
    requiresScientificValidation: true,
  },
  {
    name: "analysis.validate",
    label: "Validate Biomedical Analysis",
    description: "Run deterministic organism, build, replicate, inferential-unit, contrast, multiple-testing, artifact, and EvidenceRef checks.",
    parameters: Type.Object({
      expected: Type.Optional(Type.Record(Type.String(), Type.Unknown())),
      dataset: Type.Record(Type.String(), Type.Unknown()),
      design: Type.Optional(Type.Record(Type.String(), Type.Unknown())),
      result: Type.Record(Type.String(), Type.Unknown()),
      evidence: Type.Array(Type.Record(Type.String(), Type.Unknown()), { maxItems: 1000 }),
    }, { additionalProperties: false }),
    risk: "read",
    profiles: allProfiles,
    executionMode: "sequential",
    executionBackend: "agent-runtime",
    deterministicLocal: "analysis.validate",
  },
];

const byName = new Map(definitions.map((definition) => [definition.name, definition]));

export const KNOWN_TOOL_NAMES = new Set(byName.keys());
export const EXECUTABLE_TOOL_NAMES = new Set(
  definitions
    .filter(({ executionBackend }) => executionBackend === "os-server" || executionBackend === "agent-runtime")
    .map(({ name }) => name),
);
export const SUPPORTED_COMPUTE_PROFILES = new Set(["cpu-small", "ide-small"]);

export function getToolDefinition(name: string): GovernedToolDefinition | undefined {
  return byName.get(name);
}

export function requiredApprovalForTool(name: string): SkillSelection["permissions"]["approvals"][number] | undefined {
  switch (name) {
    case "runtime.submit_job": return "runtime.compute";
    case "project.write_file": return "project.write";
    case "runtime.cancel_job": return "runtime.cancel";
    case "artifact.register": return "artifact.register";
    default: return undefined;
  }
}

export function resolveToolDefinitions(
  profile: ToolProfile,
  selectedSkills: SkillSelection[] | undefined,
): GovernedToolDefinition[] {
  const profileTools = definitions.filter(({ profiles }) => profiles.includes(profile));
  const skillTools = new Set((selectedSkills ?? []).flatMap(({ permissions }) => permissions.tools));
  for (const name of ["skill.load", "plan.propose", "plan.update", "analysis.validate"]) skillTools.add(name);
  return profileTools.filter(({ name }) => skillTools.has(name));
}
