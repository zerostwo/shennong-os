export type JsonPrimitive = string | number | boolean | null;
export type JsonValue = JsonPrimitive | JsonValue[] | { [key: string]: JsonValue };
export type JsonObject = { [key: string]: JsonValue };

export type ProviderKind = "openai" | "deepseek" | "ollama" | "llama-cpp" | "openai-compatible";

export interface ProviderConfig {
  kind: ProviderKind;
  baseUrl: string;
  model: string;
  apiKey?: string;
  capabilities?: Array<"tools" | "thinking" | "images">;
  contextWindow?: number;
  maxTokens?: number;
  dataPolicy: "public_only" | "allow_private";
}

export interface ConversationMessage {
  id?: string;
  role: "user" | "assistant";
  content: string | JsonValue[];
  timestamp?: number;
}

export interface AgUiRunAgentInput {
  threadId: string;
  runId: string;
  parentRunId?: string;
  state?: JsonValue;
  messages: Array<{
    id?: string;
    role: string;
    content: string | JsonValue[];
    name?: string;
  }>;
  tools?: JsonValue[];
  context?: JsonValue[];
  forwardedProps?: JsonObject;
  resume?: AgUiResumeEntry[];
}

export interface AgUiResumeEntry {
  interruptId: string;
  status: "resolved" | "cancelled";
  payload?: JsonValue;
}

export type ToolProfile = "global-read" | "project-analysis" | "project-write";

export interface RunScope {
  userId: string;
  threadId: string;
  projectId?: string;
  role: "user" | "admin";
  providerDataPolicy: ProviderConfig["dataPolicy"];
}

export interface TaintedRunContext {
  memories?: JsonValue[];
  project?: JsonValue;
  datasets?: JsonValue[];
  artifacts?: JsonValue[];
  attachments?: JsonValue[];
  selectedSkills?: SkillSelection[];
  threadSummary?: string;
}

export interface SkillSelection {
  id: string;
  version: string;
  digest: string;
  loadRef?: string;
  name: string;
  description: string;
  content?: string;
  permissions: SkillPermissions;
}

export interface RunRequest {
  runId: string;
  parentRunId?: string;
  scope: RunScope;
  runCapabilityToken: string;
  provider: ProviderConfig;
  messages: ConversationMessage[];
  context?: TaintedRunContext;
  toolProfile: ToolProfile;
  thinkingLevel?: "off" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max";
  timeoutMs?: number;
  resumeApproval?: ResumeApproval;
}

export interface ResumeApproval {
  originalRunId: string;
  interruptId: string;
  status: "resolved" | "cancelled";
  toolCallId: string;
  toolName: string;
  argumentsDigest: string;
  risk: ToolRisk;
  approvalScope: "runtime.compute" | "project.write" | "runtime.cancel" | "artifact.register";
  expiresAt: string;
  arguments?: JsonObject;
  executionToken?: string;
}

export interface EvidenceRef {
  id: string;
  kind: "dataset" | "query" | "tool-result" | "artifact" | "publication";
  runId: string;
  sourceId: string;
  version?: string;
  digest?: string;
  locator?: string;
  toolCallId?: string;
  metadata?: JsonObject;
}

export type ValidationSeverity = "error" | "warning" | "info";

export interface ValidationFinding {
  code: string;
  severity: ValidationSeverity;
  message: string;
  path?: string;
}

export interface AnalysisValidationInput {
  expected?: {
    organism?: string;
    referenceBuild?: string;
    annotationRelease?: string;
  };
  dataset: {
    organism?: string;
    referenceBuild?: string;
    annotationRelease?: string;
    sampleCount: number;
    uniqueSampleCount?: number;
    patientCount?: number;
    inferentialUnit?: "cell" | "sample" | "patient";
    groupReplicates?: Record<string, number>;
  };
  design?: {
    groups?: string[];
    contrast?: [string, string];
    paired?: boolean;
    pairingKeyPresent?: boolean;
    multipleTestingRequired?: boolean;
  };
  result: {
    rowCount: number;
    effectSizePresent?: boolean;
    pValuePresent?: boolean;
    adjustedPValuePresent?: boolean;
    confidenceIntervalPresent?: boolean;
    expectedArtifactCount?: number;
    artifacts?: Array<{ id: string; sizeBytes: number; digest?: string }>;
    citationIds?: string[];
  };
  evidence: EvidenceRef[];
}

export interface AnalysisValidationReport {
  status: "pass" | "warn" | "fail";
  findings: ValidationFinding[];
  checkedAt: string;
  validatorVersion: string;
}

export type SkillLifecycle = "draft" | "validating" | "active" | "deprecated" | "disabled" | "revoked";
export type SkillScope = "platform" | "installation" | "user" | "project";

export interface SkillPermissions {
  tools: string[];
  projectRead: string[];
  projectWrite: string[];
  datasetAccess: "public" | "private";
  networkHosts: string[];
  computeProfiles: string[];
  approvals: string[];
}

export interface SkillContractField {
  name: string;
  type: string;
  required: boolean;
  description: string;
}

export interface SkillContract {
  inputs: SkillContractField[];
  outputs: SkillContractField[];
}

export interface ShennongSkillManifest {
  apiVersion: "shennong.one/v1";
  kind: "AgentSkill";
  metadata: {
    id: string;
    name: string;
    version: string;
    revision: number;
    digest: string;
    scope: SkillScope;
    publisher: string;
    trust: "built-in-reviewed" | "admin-reviewed" | "untrusted";
  };
  spec: {
    entrypoint: string;
    description: string;
    lifecycle: SkillLifecycle;
    compatibility: {
      os: string;
      runtime: string;
      dbApi: string;
      pi: string;
    };
    contract: SkillContract;
    permissions: SkillPermissions;
    validators: string[];
  };
}

export interface CapabilityVerificationRequest {
  runId: string;
  userId: string;
  projectId?: string;
  toolCallId: string;
  toolName: string;
  argumentsDigest: string;
  risk: ToolRisk;
  runCapabilityToken: string;
  arguments: JsonObject;
}

export interface CapabilityDecision {
  allowed: boolean;
  reason?: string;
  executionToken?: string;
  approvalId?: string;
  approvalScope?: string;
  expiresAt?: string;
}

export type ToolRisk = "read" | "write" | "network" | "compute" | "destructive" | "admin";

export interface BackendToolExecutionRequest extends CapabilityVerificationRequest {
  executionToken: string;
}

export interface BackendToolExecutionResult {
  content: JsonValue;
  evidence?: EvidenceRef[];
  activity?: JsonValue;
}

export interface PersistedRunMetadata {
  runId: string;
  parentRunId?: string;
  userId: string;
  threadId: string;
  projectId?: string;
  model: string;
  provider: string;
  promptDigest: string;
  platformPolicyVersion: string;
  biomedicalPolicyVersion: string;
  skills: Array<{ id: string; version: string; digest: string }>;
}

export interface RunResult {
  runId: string;
  content: string;
  evidence: EvidenceRef[];
  validationReports: AnalysisValidationReport[];
  stopReason: string;
}

export interface AgUiEvent {
  type: string;
  timestamp?: number;
  [key: string]: JsonValue | undefined;
}
