import assert from "node:assert/strict";
import test from "node:test";
import { HttpAgent } from "@ag-ui/client";
import type { StreamFn } from "@earendil-works/pi-agent-core";
import { createAssistantMessageEventStream } from "@earendil-works/pi-ai";
import type { AssistantMessage, StopReason, ToolResultMessage } from "@earendil-works/pi-ai";
import { encodeSse } from "./ag-ui.js";
import { ShennongAgentHarness } from "./harness.js";
import type { AgentRunIdentity, AgentUserMessageInput, OsInternalClient } from "./os-client.js";
import { argumentsDigest } from "./security.js";
import type {
  AgUiEvent,
  BackendToolExecutionRequest,
  BackendToolExecutionResult,
  CapabilityDecision,
  CapabilityVerificationRequest,
  PersistedRunMetadata,
  RunRequest,
  RunResult,
} from "./types.js";

function zeroUsage(): AssistantMessage["usage"] {
  return {
    input: 0,
    output: 0,
    cacheRead: 0,
    cacheWrite: 0,
    totalTokens: 0,
    cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
  };
}

const approvedJobArguments = {
  plan_step_id: "step-approved",
  job_spec: { argv: ["python3", "analysis.py"], worker_profile: "cpu-small" },
};

function resumedRun(status: "resolved" | "cancelled" = "resolved"): RunRequest {
  const run = runRequest();
  run.runId = "run-resumed";
  run.parentRunId = "run-original";
  run.resumeApproval = {
    originalRunId: "run-original",
    interruptId: "approval-1",
    status,
    toolCallId: "tool-approved",
    toolName: "runtime.submit_job",
    argumentsDigest: argumentsDigest("runtime.submit_job", approvedJobArguments),
    risk: "compute",
    approvalScope: "runtime.compute",
    expiresAt: new Date(Date.now() + 60_000).toISOString(),
    ...(status === "resolved" ? {
      arguments: approvedJobArguments,
      executionToken: "approved-execution-token-value-1234567890",
    } : {}),
  };
  return run;
}

function assistant(content: AssistantMessage["content"], stopReason: StopReason, errorMessage?: string): AssistantMessage {
  return {
    role: "assistant",
    content,
    api: "openai-completions",
    provider: "openai",
    model: "test-model",
    usage: zeroUsage(),
    stopReason,
    ...(errorMessage ? { errorMessage } : {}),
    timestamp: Date.now(),
  };
}

function terminal(message: AssistantMessage): ReturnType<typeof createAssistantMessageEventStream> {
  const stream = createAssistantMessageEventStream();
  queueMicrotask(() => {
    if (message.stopReason === "error" || message.stopReason === "aborted") {
      stream.push({ type: "error", reason: message.stopReason, error: message });
    } else {
      stream.push({ type: "done", reason: message.stopReason, message });
    }
  });
  return stream;
}

function runRequest(apiKey = "provider-secret-value"): RunRequest {
  return {
    runId: "run-harness",
    scope: {
      userId: "user-1",
      threadId: "thread-1",
      projectId: "project-1",
      role: "user",
      providerDataPolicy: "allow_private",
    },
    runCapabilityToken: "r".repeat(48),
    provider: {
      kind: "openai",
      baseUrl: "https://provider.example/v1",
      model: "test-model",
      apiKey,
      dataPolicy: "allow_private",
    },
    messages: [{ role: "user", content: "Run the analysis." }],
    context: {
      selectedSkills: [{
        id: "zerostwo/run-shennong-single-cell-workflow",
        version: "1.0.0",
        digest: `sha256:${"a".repeat(64)}`,
        name: "run-shennong-single-cell-workflow",
        description: "Run a governed single-cell workflow.",
        permissions: {
          tools: ["runtime.submit_job", "analysis.validate"],
          projectRead: ["project://current/"],
          projectWrite: ["project://current/results/"],
          datasetAccess: "private",
          networkHosts: [],
          computeProfiles: ["cpu-small"],
          approvals: ["runtime.compute"],
        },
      }],
    },
    toolProfile: "project-analysis",
  };
}

class RecordingOsClient implements OsInternalClient {
  events: AgUiEvent[] = [];
  verifications: CapabilityVerificationRequest[] = [];
  executions: BackendToolExecutionRequest[] = [];
  finishes: Array<{ result?: RunResult; error?: { code: string; message: string } }> = [];
  verificationDecision: CapabilityDecision = { allowed: true, executionToken: "execution-token-value" };
  executionHandler?: (request: BackendToolExecutionRequest) => BackendToolExecutionResult;

  async bootstrap(_identity: AgentRunIdentity): Promise<RunRequest> { throw new Error("unused"); }
  async persistLatestUserMessage(_input: AgentUserMessageInput): Promise<void> {}
  async recordRunMetadata(_metadata: PersistedRunMetadata): Promise<void> {}
  async appendEvent(_runId: string, event: AgUiEvent): Promise<string | undefined> {
    this.events.push(event);
    return String(this.events.length);
  }
  async finishRun(
    _runId: string,
    result: RunResult | undefined,
    error?: { code: string; message: string },
  ): Promise<void> {
    this.finishes.push({ ...(result ? { result } : {}), ...(error ? { error } : {}) });
  }
  async verifyCapability(request: CapabilityVerificationRequest): Promise<CapabilityDecision> {
    this.verifications.push(request);
    return this.verificationDecision;
  }
  async executeTool(request: BackendToolExecutionRequest): Promise<BackendToolExecutionResult> {
    this.executions.push(request);
    if (this.executionHandler) return this.executionHandler(request);
    return {
      content: { jobId: "job-1", status: "completed" },
      evidence: [{
        id: "ev-job",
        kind: "artifact",
        runId: request.runId,
        sourceId: "job-1",
        digest: `sha256:${"a".repeat(64)}`,
        metadata: { untrustedMarker: "must-not-reach-provider-tool-content" },
      }, {
        id: "ev-cross-run",
        kind: "artifact",
        runId: "another-run",
        sourceId: "job-cross-run",
        digest: `sha256:${"b".repeat(64)}`,
      }],
    };
  }
}

test("all Runtime execution crosses both OS capability and execution callbacks", async () => {
  const osClient = new RecordingOsClient();
  let calls = 0;
  let providerToolResult = "";
  const streamFn: StreamFn = (_model, context) => {
    calls += 1;
    if (calls === 1) {
      return terminal(assistant([
        {
          type: "toolCall",
          id: "tool-1",
          name: "runtime.submit_job",
          arguments: { plan_step_id: "step-1", job_spec: { token: "exact-value", method: "single-cell" } },
        },
      ], "toolUse"));
    }
    providerToolResult = context.messages
      .filter((message): message is ToolResultMessage => message.role === "toolResult")
      .flatMap(({ content }) => content)
      .filter((part): part is Extract<ToolResultMessage["content"][number], { type: "text" }> => part.type === "text")
      .map(({ text }) => text)
      .join("\n");
    return terminal(assistant([{ type: "text", text: "The job completed [EvidenceRef:ev-job]." }], "stop"));
  };
  const result = await new ShennongAgentHarness({ osClient, streamFn }).run(
    runRequest(),
    (event, cursor) => { encodeSse(event, cursor); },
  );

  assert.equal(osClient.verifications.length, 1);
  assert.equal(osClient.executions.length, 1);
  assert.equal(osClient.verifications[0]?.argumentsDigest, osClient.executions[0]?.argumentsDigest);
  assert.match(providerToolResult, /<trusted_evidence_ids encoding="escaped-json">\n\["ev-job"\]\n<\/trusted_evidence_ids>/);
  assert.doesNotMatch(providerToolResult, /ev-cross-run|must-not-reach-provider-tool-content/);
  assert.deepEqual(osClient.executions[0]?.arguments, {
    job_spec: { method: "single-cell", token: "exact-value" },
    plan_step_id: "step-1",
  });
  assert.equal(result.evidence[0]?.id, "ev-job");
  assert.equal(result.stopReason, "validation_failed");
  assert.ok(result.validationReports.some(({ findings }) =>
    findings.some(({ code }) => code === "analysis_validation_missing")));
  assert.equal(osClient.finishes[0]?.result?.stopReason, "validation_failed");
  assert.equal(osClient.events.at(-1)?.type, "RUN_ERROR");
  assert.equal(osClient.events.at(-1)?.code, "analysis_validation_failed");

  const protocolClient = new HttpAgent({
    url: "http://agent-runtime.invalid/v1/agent",
    threadId: "thread-1",
    fetch: async () => new Response(
      osClient.events.map((event, index) => encodeSse(event, String(index + 1))).join(""),
      { status: 200, headers: { "content-type": "text/event-stream" } },
    ),
  });
  protocolClient.addMessage({ id: "user-protocol", role: "user", content: "Run the analysis." });
  await protocolClient.runAgent({ runId: "run-harness" });
});

test("provider errors become redacted RUN_ERROR events and never fake success", async () => {
  const osClient = new RecordingOsClient();
  const apiKey = "provider-secret-value";
  const streamFn: StreamFn = () => terminal(assistant([], "error", `Upstream rejected ${apiKey}`));
  const harness = new ShennongAgentHarness({ osClient, streamFn });

  await assert.rejects(harness.run(
    runRequest(apiKey),
    (event, cursor) => { encodeSse(event, cursor); },
  ), /\[redacted\]/);
  const errors = osClient.events.filter(({ type }) => type === "RUN_ERROR");
  assert.equal(errors.length, 1);
  assert.equal(errors[0]?.code, "provider_error");
  assert.doesNotMatch(String(errors[0]?.message), /provider-secret-value/);
  assert.equal(osClient.events.some(({ type }) => type === "RUN_FINISHED"), false);
  assert.equal(osClient.finishes[0]?.error?.code, "provider_error");
  assert.doesNotMatch(osClient.finishes[0]?.error?.message ?? "", /provider-secret-value/);
});

test("a selected biomedical Skill cannot return an unsupported model-only conclusion", async () => {
  const osClient = new RecordingOsClient();
  const streamFn: StreamFn = () => terminal(assistant([
    { type: "text", text: "The treatment changes the disease mechanism." },
  ], "stop"));
  const result = await new ShennongAgentHarness({ osClient, streamFn }).run(runRequest());

  assert.equal(result.stopReason, "validation_failed");
  assert.equal(result.evidence.length, 0);
  const codes = result.validationReports.flatMap(({ findings }) => findings.map(({ code }) => code));
  assert.ok(codes.includes("analysis_validation_missing"));
  assert.ok(codes.includes("evidence_missing"));
  assert.equal(osClient.events.at(-1)?.type, "RUN_ERROR");
  assert.equal(osClient.events.some(({ type }) => type === "RUN_FINISHED"), false);
});

test("a personal chat can complete a governed public Resource query with Skill guidance", async () => {
  const osClient = new RecordingOsClient();
  osClient.executionHandler = (request) => {
    const evidenceId = request.toolName === "db.query_resource"
      ? "ev-query"
      : `ev-${request.toolName.split(".").at(-1)}`;
    return {
      content: request.toolName === "db.discover_resources"
        ? { data: [{ id: "toil", metadata: { cohort: "TCGA GTEx TOIL" } }], discovery: { fallback: "bounded_public_catalog" } }
        : request.toolName === "db.query_resource"
          ? { data: { rows: [{ cohort: "TCGA COAD", group: "Primary Tumor", median: 5.24 }, { cohort: "TCGA COAD", group: "Adjacent Normal", median: 4.99 }] } }
          : { data: { id: "toil", permissions: { visibility: "public" } } },
      evidence: [{
        id: evidenceId,
        kind: request.toolName === "db.query_resource" || request.toolName === "db.discover_resources" ? "query" : "dataset",
        runId: request.runId,
        sourceId: "toil",
        digest: `sha256:${"b".repeat(64)}`,
      }],
    };
  };
  const run = runRequest();
  delete run.scope.projectId;
  run.toolProfile = "global-read";
  run.messages = [{ role: "user", content: "YTHDF2在结肠癌中是上调的吗？" }];
  run.context = {
    selectedSkills: [{
      id: "zerostwo/discover-shennong-data",
      version: "1.0.0",
      digest: `sha256:${"a".repeat(64)}`,
      loadRef: "4b23d46a-ac8a-544f-8492-7f461b76e293:1",
      name: "discover-shennong-data",
      description: "Discover governed public Resources.",
      content: "Discover broadly, inspect, query the declared operation, get provenance, validate, and cite EvidenceRefs.",
      permissions: {
        tools: ["db.discover_resources", "db.inspect_resource", "db.query_resource", "db.get_provenance", "analysis.validate"],
        projectRead: [],
        projectWrite: [],
        datasetAccess: "public",
        networkHosts: [],
        computeProfiles: [],
        approvals: [],
      },
    }],
  };
  const calls = [
    { id: "discover", name: "db.discover_resources", arguments: { q: "colon adenocarcinoma expression", limit: 20 } },
    { id: "inspect", name: "db.inspect_resource", arguments: { resource: "toil" } },
    { id: "query", name: "db.query_resource", arguments: { resource: "toil", operation: "expression", feature: "YTHDF2", context: { primary_site: "Colon" }, limit: 1000 } },
    { id: "provenance", name: "db.get_provenance", arguments: { resource: "toil" } },
    {
      id: "validate",
      name: "analysis.validate",
      arguments: {
        dataset: { sampleCount: 329, uniqueSampleCount: 329, inferentialUnit: "sample", groupReplicates: { tumor: 288, normal: 41 } },
        design: { groups: ["tumor", "normal"], contrast: ["tumor", "normal"] },
        result: { rowCount: 2, effectSizePresent: true, citationIds: ["ev-query"] },
        evidence: [],
      },
    },
  ];
  let providerCall = 0;
  const streamFn: StreamFn = () => {
    const call = calls[providerCall++];
    if (call) {
      return terminal(assistant([{ type: "toolCall", ...call }], "toolUse"));
    }
    return terminal(assistant([{
      type: "text",
      text: "TOIL中YTHDF2在COAD原发肿瘤的中位表达高于癌旁正常，但差异幅度不大 [evidence:ev-query]。",
    }], "stop"));
  };

  const result = await new ShennongAgentHarness({ osClient, streamFn }).run(run);

  assert.equal(result.stopReason, "stop");
  assert.deepEqual(osClient.executions.map(({ toolName }) => toolName), [
    "db.discover_resources",
    "db.inspect_resource",
    "db.query_resource",
    "db.get_provenance",
  ]);
  assert.equal(osClient.verifications.length, 5);
  assert.equal(result.validationReports.some(({ status }) => status === "fail"), false);
  assert.equal(osClient.events.at(-1)?.type, "RUN_FINISHED");
});

test("OS-issued run evidence plus deterministic validation is required for scientific success", async () => {
  const osClient = new RecordingOsClient();
  let calls = 0;
  const streamFn: StreamFn = () => {
    calls += 1;
    if (calls === 1) {
      return terminal(assistant([{
        type: "toolCall",
        id: "tool-runtime",
        name: "runtime.submit_job",
        arguments: { plan_step_id: "step-1", job_spec: { argv: ["python3", "analysis.py"] } },
      }], "toolUse"));
    }
    if (calls === 2) {
      return terminal(assistant([{
        type: "toolCall",
        id: "tool-validator",
        name: "analysis.validate",
        arguments: {
          dataset: { sampleCount: 6, uniqueSampleCount: 6, inferentialUnit: "sample", groupReplicates: { case: 3, control: 3 } },
          design: { groups: ["case", "control"], contrast: ["case", "control"], multipleTestingRequired: true },
          result: { rowCount: 10, effectSizePresent: true, adjustedPValuePresent: true, citationIds: ["ev-job"] },
          evidence: [{ id: "fabricated", kind: "artifact", runId: "run-harness", sourceId: "fake" }],
        },
      }], "toolUse"));
    }
    return terminal(assistant([{ type: "text", text: "Validated result [EvidenceRef:ev-job]." }], "stop"));
  };

  const result = await new ShennongAgentHarness({ osClient, streamFn }).run(runRequest());

  assert.equal(result.stopReason, "stop");
  assert.deepEqual(result.evidence.map(({ id }) => id), ["ev-job"]);
  assert.equal(result.validationReports.some(({ status }) => status === "fail"), false);
  assert.equal(result.validationReports.length, 1);
  assert.equal(osClient.executions.length, 1);
  assert.equal(osClient.verifications.length, 2);
});

test("approval-required tools finish with a native durable interrupt and never execute", async () => {
  const osClient = new RecordingOsClient();
  let providerCalls = 0;
  const expiresAt = new Date(Date.now() + 60_000).toISOString();
  osClient.verificationDecision = {
    allowed: false,
    reason: "approval_required",
    approvalId: "approval-1",
    approvalScope: "runtime.compute",
    expiresAt,
  };
  const streamFn: StreamFn = () => {
    providerCalls += 1;
    return terminal(assistant([{
      type: "toolCall",
      id: "tool-approved",
      name: "runtime.submit_job",
      arguments: approvedJobArguments,
    }], "toolUse"));
  };

  const result = await new ShennongAgentHarness({ osClient, streamFn }).run(
    runRequest(),
    (event, cursor) => { encodeSse(event, cursor); },
  );

  assert.equal(result.stopReason, "interrupted");
  assert.equal(osClient.verifications.length, 1);
  assert.equal(providerCalls, 1);
  assert.deepEqual(osClient.verifications[0]?.arguments, approvedJobArguments);
  assert.equal(osClient.executions.length, 0);
  assert.equal(osClient.finishes.length, 0);
  assert.equal(osClient.events.some((event) =>
    event.type === "TOOL_CALL_RESULT" && event.toolCallId === "tool-approved"), false);

  const finished = osClient.events.at(-1);
  assert.equal(finished?.type, "RUN_FINISHED");
  if (finished?.type !== "RUN_FINISHED") throw new Error("missing native interrupt");
  const outcome = finished.outcome as { type?: string; interrupts?: unknown[] } | undefined;
  assert.equal(outcome?.type, "interrupt");
  if (outcome?.type !== "interrupt" || !outcome.interrupts) throw new Error("missing native interrupt outcome");
  assert.equal(outcome.interrupts.length, 1);
  const interrupt = outcome.interrupts[0] as Record<string, unknown>;
  assert.equal(interrupt.id, "approval-1");
  assert.equal(interrupt.toolCallId, "tool-approved");
  assert.equal(interrupt.expiresAt, expiresAt);
  assert.deepEqual(interrupt.responseSchema, {
    type: "object",
    properties: { approved: { const: true } },
    required: ["approved"],
    additionalProperties: false,
  });
  const metadata = interrupt.metadata as Record<string, unknown>;
  assert.equal(metadata.originalRunId, "run-harness");
  assert.equal(metadata.approvalScope, "runtime.compute");
  assert.equal(metadata.argumentsDigest, argumentsDigest("runtime.submit_job", approvedJobArguments));
  encodeSse(finished, String(osClient.events.length));
});

test("an approved native resume executes the immutable tool call exactly once before continuing", async () => {
  const osClient = new RecordingOsClient();
  const observedContexts: Array<unknown[]> = [];
  let calls = 0;
  const streamFn: StreamFn = (_model, context) => {
    calls += 1;
    observedContexts.push([...context.messages]);
    if (calls === 1) {
      return terminal(assistant([{
        type: "toolCall",
        id: "tool-validator",
        name: "analysis.validate",
        arguments: {
          dataset: {
            sampleCount: 6,
            uniqueSampleCount: 6,
            inferentialUnit: "sample",
            groupReplicates: { case: 3, control: 3 },
          },
          design: {
            groups: ["case", "control"],
            contrast: ["case", "control"],
            multipleTestingRequired: true,
          },
          result: {
            rowCount: 10,
            effectSizePresent: true,
            adjustedPValuePresent: true,
            citationIds: ["ev-job"],
          },
          evidence: [],
        },
      }], "toolUse"));
    }
    return terminal(assistant([{
      type: "text",
      text: "The approved computation completed [EvidenceRef:ev-job].",
    }], "stop"));
  };

  const result = await new ShennongAgentHarness({ osClient, streamFn }).run(
    resumedRun(),
    (event, cursor) => { encodeSse(event, cursor); },
  );

  assert.equal(result.stopReason, "stop");
  assert.equal(osClient.executions.length, 1);
  assert.deepEqual(osClient.executions[0], {
    runId: "run-resumed",
    userId: "user-1",
    projectId: "project-1",
    toolCallId: "tool-approved",
    toolName: "runtime.submit_job",
    argumentsDigest: argumentsDigest("runtime.submit_job", approvedJobArguments),
    risk: "compute",
    runCapabilityToken: "r".repeat(48),
    arguments: approvedJobArguments,
    executionToken: "approved-execution-token-value-1234567890",
  });
  assert.equal(osClient.verifications.length, 1);
  assert.equal(osClient.verifications[0]?.toolName, "analysis.validate");
  assert.equal(osClient.finishes.length, 1);
  assert.equal(osClient.events.at(-1)?.type, "RUN_FINISHED");
  assert.ok(observedContexts[0]?.some((message) =>
    typeof message === "object" && message !== null &&
    (message as { role?: string }).role === "assistant" &&
    Array.isArray((message as { content?: unknown }).content) &&
    (message as { content: Array<{ type?: string; id?: string }> }).content.some((part) =>
      part.type === "toolCall" && part.id === "tool-approved")));
  assert.ok(observedContexts[0]?.some((message) =>
    typeof message === "object" && message !== null &&
    (message as { role?: string }).role === "toolResult" &&
    (message as { toolCallId?: string }).toolCallId === "tool-approved"));
});

test("a rejected native resume cancels without provider or tool execution", async () => {
  const osClient = new RecordingOsClient();
  let providerCalls = 0;
  const streamFn: StreamFn = () => {
    providerCalls += 1;
    return terminal(assistant([{ type: "text", text: "must not run" }], "stop"));
  };

  const result = await new ShennongAgentHarness({ osClient, streamFn }).run(resumedRun("cancelled"));

  assert.equal(result.stopReason, "approval_rejected");
  assert.equal(providerCalls, 0);
  assert.equal(osClient.verifications.length, 0);
  assert.equal(osClient.executions.length, 0);
  assert.equal(osClient.events.at(-1)?.type, "RUN_FINISHED");
  assert.deepEqual(osClient.events.at(-1)?.result, { stopReason: "approval_rejected" });
  assert.equal(osClient.finishes[0]?.result?.stopReason, "approval_rejected");
});

test("an approved native resume rejects tampered arguments before execution", async () => {
  const osClient = new RecordingOsClient();
  const run = resumedRun();
  if (!run.resumeApproval) throw new Error("missing resume fixture");
  run.resumeApproval.arguments = {
    ...approvedJobArguments,
    job_spec: { argv: ["python3", "tampered.py"], worker_profile: "cpu-small" },
  };

  await assert.rejects(
    new ShennongAgentHarness({
      osClient,
      streamFn: () => terminal(assistant([{ type: "text", text: "must not run" }], "stop")),
    }).run(run),
    /immutable digest/,
  );

  assert.equal(osClient.executions.length, 0);
  assert.equal(osClient.events.at(-1)?.type, "RUN_ERROR");
  assert.equal(osClient.finishes[0]?.error?.code, "approval_resume_invalid");
});
