import { AsyncLocalStorage } from "node:async_hooks";
import { Agent, type AgentMessage, type AgentTool, type StreamFn } from "@earendil-works/pi-agent-core";
import { createAssistantMessageEventStream, streamSimple } from "@earendil-works/pi-ai/compat";
import type { AssistantMessage, Model, ToolResultMessage } from "@earendil-works/pi-ai";
import { AgUiEventMapper } from "./ag-ui.js";
import { validateAnalysis } from "./analysis-validator.js";
import { isEvidenceRef, validateCitationIds } from "./evidence.js";
import { createProviderFetchGuard, type ProviderFetchPolicy } from "./fetch-guard.js";
import type { OsInternalClient } from "./os-client.js";
import { PromptCompiler } from "./prompt-compiler.js";
import {
  argumentsDigest,
  canonicalJson,
  canonicalToolArguments,
  promptSafeJson,
  redactText,
  sanitizeUntrusted,
} from "./security.js";
import {
  requiredApprovalForTool,
  resolveToolDefinitions,
  type GovernedToolDefinition,
} from "./tool-registry.js";
import { ToolPolicyEngine } from "./tool-policy.js";
import type {
  AgUiEvent,
  AnalysisValidationInput,
  AnalysisValidationReport,
  BackendToolExecutionResult,
  CapabilityDecision,
  ConversationMessage,
  EvidenceRef,
  JsonObject,
  JsonValue,
  PersistedRunMetadata,
  ResumeApproval,
  RunRequest,
  RunResult,
} from "./types.js";

const MAX_HISTORY_MESSAGES = 80;
const MAX_HISTORY_BYTES = 512 * 1024;

export type AgUiSink = (event: AgUiEvent, cursor?: string) => Promise<void> | void;

export class HarnessError extends Error {
  constructor(
    readonly code: string,
    message: string,
  ) {
    super(message);
  }
}

export class ProviderFetchScope {
  private readonly storage = new AsyncLocalStorage<ProviderFetchPolicy>();
  private installed = false;

  install(): void {
    if (this.installed) return;
    globalThis.fetch = createProviderFetchGuard({ getPolicy: () => this.storage.getStore() });
    this.installed = true;
  }

  async run<T>(policy: ProviderFetchPolicy, task: () => Promise<T>): Promise<T> {
    return await this.storage.run(policy, task);
  }
}

function messageText(content: ConversationMessage["content"]): string {
  if (typeof content === "string") return content;
  return promptSafeJson(content);
}

function zeroUsage() {
  return {
    input: 0,
    output: 0,
    cacheRead: 0,
    cacheWrite: 0,
    totalTokens: 0,
    cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
  };
}

function historyMessage(message: ConversationMessage, run: RunRequest): AgentMessage {
  if (message.role === "user") {
    return { role: "user", content: messageText(message.content), timestamp: message.timestamp ?? Date.now() };
  }
  const assistant: AssistantMessage = {
    role: "assistant",
    content: [{ type: "text", text: messageText(message.content) }],
    api: "openai-completions",
    provider: run.provider.kind,
    model: run.provider.model,
    usage: zeroUsage(),
    stopReason: "stop",
    timestamp: message.timestamp ?? Date.now(),
  };
  return assistant;
}

function boundedContext(messages: AgentMessage[]): AgentMessage[] {
  const selected: AgentMessage[] = [];
  let bytes = 0;
  for (const message of messages.slice(-MAX_HISTORY_MESSAGES).reverse()) {
    const size = Buffer.byteLength(canonicalJson(message));
    if (bytes + size > MAX_HISTORY_BYTES) break;
    bytes += size;
    selected.push(message);
  }
  return selected.reverse();
}

function customModel(run: RunRequest): Model<"openai-completions"> {
  return {
    id: run.provider.model,
    name: run.provider.model,
    api: "openai-completions",
    provider: run.provider.kind,
    baseUrl: run.provider.baseUrl.replace(/\/$/, ""),
    reasoning: run.provider.capabilities?.includes("thinking") ?? run.provider.kind === "deepseek",
    input: ["text"],
    cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
    contextWindow: run.provider.contextWindow ?? 128_000,
    maxTokens: run.provider.maxTokens ?? 16_384,
  };
}

function finalAssistant(messages: AgentMessage[]): AssistantMessage | undefined {
  return [...messages].reverse().find((message): message is AssistantMessage => message.role === "assistant");
}

function finalAssistantText(messages: AgentMessage[]): string {
  const assistant = finalAssistant(messages);
  if (!assistant) return "";
  return assistant.content
    .filter((content): content is Extract<typeof content, { type: "text" }> => content.type === "text")
    .map(({ text }) => text)
    .join("")
    .trim();
}

function citedEvidenceIds(text: string): string[] {
  const ids = new Set<string>();
  for (const match of text.matchAll(/\[(?:evidence|EvidenceRef):([A-Za-z0-9][A-Za-z0-9._:-]{0,127})\]/g)) {
    const id = match[1];
    if (id) ids.add(id);
  }
  return [...ids];
}

function answerValidation(
  text: string,
  evidence: EvidenceRef[],
  requireEvidence: boolean,
): AnalysisValidationReport | undefined {
  const citationIds = citedEvidenceIds(text);
  const findings = validateCitationIds(citationIds, evidence);
  if (requireEvidence && evidence.length === 0) {
    findings.push({
      code: "evidence_missing",
      severity: "error",
      message: "A scientific run completed without any backend-issued EvidenceRef.",
      path: "evidence",
    });
  } else if (requireEvidence && citationIds.length === 0) {
    findings.push({
      code: "citation_missing",
      severity: "error",
      message: "A scientific answer did not cite any backend-issued EvidenceRef.",
      path: "content",
    });
  }
  if (!findings.length) return undefined;
  return {
    status: "fail",
    findings,
    checkedAt: new Date().toISOString(),
    validatorVersion: "shennong-evidence-validator/v1",
  };
}

function toJsonObject(value: unknown): JsonObject {
  const canonical = canonicalToolArguments(value);
  const decoded = JSON.parse(canonical) as unknown;
  if (!decoded || typeof decoded !== "object" || Array.isArray(decoded)) {
    throw new Error("tool_arguments_invalid");
  }
  return decoded as JsonObject;
}

function approvalDeclared(run: RunRequest, toolName: string, approvalScope: string): boolean {
  return (run.context?.selectedSkills ?? []).some(({ permissions }) =>
    permissions.tools.includes(toolName) && permissions.approvals.includes(approvalScope));
}

function validateApprovedResume(
  run: RunRequest,
  resume: ResumeApproval,
  definitions: GovernedToolDefinition[],
): { definition: GovernedToolDefinition; arguments: JsonObject; executionToken: string } {
  const definition = definitions.find(({ name }) => name === resume.toolName);
  const approvalScope = requiredApprovalForTool(resume.toolName);
  if (
    resume.status !== "resolved" ||
    !run.parentRunId || run.parentRunId !== resume.originalRunId ||
    !definition || definition.risk !== resume.risk ||
    !approvalScope || approvalScope !== resume.approvalScope ||
    !approvalDeclared(run, resume.toolName, approvalScope) ||
    !resume.executionToken || resume.executionToken.length < 32 ||
    !resume.arguments
  ) {
    throw new HarnessError("approval_resume_invalid", "The approved continuation does not match the original tool contract.");
  }
  const args = toJsonObject(resume.arguments);
  if (argumentsDigest(resume.toolName, args) !== resume.argumentsDigest) {
    throw new HarnessError("approval_resume_invalid", "The approved continuation arguments do not match their immutable digest.");
  }
  return { definition, arguments: args, executionToken: resume.executionToken };
}

function resumedAssistantMessage(run: RunRequest, resume: ResumeApproval, args: JsonObject): AssistantMessage {
  return {
    role: "assistant",
    content: [{ type: "toolCall", id: resume.toolCallId, name: resume.toolName, arguments: args }],
    api: "openai-completions",
    provider: run.provider.kind,
    model: run.provider.model,
    usage: zeroUsage(),
    stopReason: "toolUse",
    timestamp: Date.now(),
  };
}

interface ExecutionState {
  decisions: Map<string, CapabilityDecision>;
  evidence: Map<string, EvidenceRef>;
  validationReports: AnalysisValidationReport[];
  scientificEpoch: number;
  validatedEpoch: number;
  pendingApproval?: {
    id: string;
    toolCallId: string;
    toolName: string;
    argumentsDigest: string;
    risk: GovernedToolDefinition["risk"];
    approvalScope: string;
    expiresAt: string;
  };
}

export interface HarnessOptions {
  osClient: OsInternalClient;
  promptCompiler?: PromptCompiler;
  policyEngine?: ToolPolicyEngine;
  providerFetchScope?: ProviderFetchScope;
  streamFn?: StreamFn;
}

export class ShennongAgentHarness {
  private readonly promptCompiler: PromptCompiler;
  private readonly policyEngine: ToolPolicyEngine;
  private readonly providerFetchScope: ProviderFetchScope;

  constructor(private readonly options: HarnessOptions) {
    this.promptCompiler = options.promptCompiler ?? new PromptCompiler();
    this.policyEngine = options.policyEngine ?? new ToolPolicyEngine();
    this.providerFetchScope = options.providerFetchScope ?? new ProviderFetchScope();
  }

  private createTool(
    run: RunRequest,
    definition: GovernedToolDefinition,
    state: ExecutionState,
  ): AgentTool {
    return {
      name: definition.name,
      label: definition.label,
      description: definition.description,
      parameters: definition.parameters,
      executionMode: definition.executionMode,
      execute: async (toolCallId, rawArguments) => {
        const decision = state.decisions.get(toolCallId);
        if (!decision?.allowed) throw new Error(decision?.reason ?? "capability_not_verified");
        const args = toJsonObject(rawArguments);
        const verifiedEvidence = new Map<string, EvidenceRef>();
        let result: BackendToolExecutionResult;
        if (definition.deterministicLocal === "analysis.validate") {
          // Evidence is an OS-issued capability, never model-supplied input. Replacing
          // the untrusted field also prevents a provider from validating fabricated IDs.
          const validationInput = {
            ...args,
            evidence: [...state.evidence.values()],
          } as unknown as AnalysisValidationInput;
          const report = validateAnalysis(validationInput);
          state.validationReports.push(report);
          state.validatedEpoch = state.scientificEpoch;
          result = { content: report as unknown as JsonValue, activity: { type: "analysis-validation", status: report.status } };
        } else {
          if (!decision.executionToken) throw new Error("execution_token_missing");
          result = await this.options.osClient.executeTool({
            runId: run.runId,
            userId: run.scope.userId,
            ...(run.scope.projectId ? { projectId: run.scope.projectId } : {}),
            toolCallId,
            toolName: definition.name,
            argumentsDigest: argumentsDigest(definition.name, args),
            risk: definition.risk,
            runCapabilityToken: run.runCapabilityToken,
            arguments: args,
            executionToken: decision.executionToken,
          });
          for (const evidence of result.evidence ?? []) {
            if (isEvidenceRef(evidence) && evidence.runId === run.runId) {
              state.evidence.set(evidence.id, evidence);
              verifiedEvidence.set(evidence.id, evidence);
            }
          }
          if (definition.requiresScientificValidation) state.scientificEpoch += 1;
        }
        const evidence = [...verifiedEvidence.values()];
        const evidenceNotice = evidence.length
          ? `\n<trusted_evidence_ids encoding="escaped-json">\n${promptSafeJson(evidence.map(({ id }) => id))}\n</trusted_evidence_ids>`
          : "";
        return {
          content: [
            {
              type: "text",
              text: `<untrusted_tool_result tool="${definition.name}" encoding="escaped-json">\n${promptSafeJson(result.content)}\n</untrusted_tool_result>${evidenceNotice}`,
            },
          ],
          details: sanitizeUntrusted({
            tool: definition.name,
            content: result.content,
            evidence,
            activity: result.activity ?? null,
          }),
        };
      },
    };
  }

  async run(run: RunRequest, sink?: AgUiSink): Promise<RunResult> {
    this.validateRun(run);
    const prompt = this.promptCompiler.compile(run);
    const mapper = new AgUiEventMapper(run.scope.threadId, run.runId);
    const executionState: ExecutionState = {
      decisions: new Map(),
      evidence: new Map(),
      validationReports: [],
      // Selecting a governed biomedical Skill makes the run scientific even if
      // a provider tries to bypass every tool and answer from model memory.
      scientificEpoch: (run.context?.selectedSkills?.length ?? 0) > 0 ? 1 : 0,
      validatedEpoch: 0,
    };
    const emit = async (event: AgUiEvent): Promise<void> => {
      const cursor = await this.options.osClient.appendEvent(run.runId, event);
      await sink?.(event, cursor);
    };
    await this.options.osClient.recordRunMetadata(this.metadata(run));
    await emit(mapper.started());

    const definitions = resolveToolDefinitions(run.toolProfile, run.context?.selectedSkills);
    const tools = definitions.map((definition) => this.createTool(run, definition, executionState));
    const definitionByName = new Map(definitions.map((definition) => [definition.name, definition]));
    const resumeApproval = run.resumeApproval;
    if (resumeApproval?.status === "cancelled") {
      const result: RunResult = {
        runId: run.runId,
        content: "",
        evidence: [],
        validationReports: [],
        stopReason: "approval_rejected",
      };
      // AG-UI 0.0.57 has no wire-level RUN_CANCELLED event. User rejection is
      // a successful, non-executing child continuation with an explicit result.
      await emit(mapper.finished({ stopReason: "approval_rejected" }));
      await this.options.osClient.finishRun(run.runId, result);
      return result;
    }
    const prior = (resumeApproval ? run.messages : run.messages.slice(0, -1))
      .map((message) => historyMessage(message, run));
    const current = run.messages.at(-1);
    if (!resumeApproval && (!current || current.role !== "user")) {
      throw new HarnessError("user_message_missing", "The last message must be from the user.");
    }

    const model = customModel(run);
    const providerStream: StreamFn = this.options.streamFn ?? streamSimple;
    const interruptibleStream: StreamFn = (streamModel, context, options) => {
      if (!executionState.pendingApproval) return providerStream(streamModel, context, options);
      // Pi's public tool-block hook turns a denial into a tool result and asks
      // the provider for another turn. Once OS has requested approval, replace
      // that extra provider call with a local aborted terminal message so the
      // durable native interrupt can be emitted without another model request.
      const stream = createAssistantMessageEventStream();
      const paused: AssistantMessage = {
        role: "assistant",
        content: [],
        api: streamModel.api,
        provider: streamModel.provider,
        model: streamModel.id,
        usage: zeroUsage(),
        stopReason: "aborted",
        errorMessage: "Approval required",
        timestamp: Date.now(),
      };
      queueMicrotask(() => stream.push({ type: "error", reason: "aborted", error: paused }));
      return stream;
    };
    const agent = new Agent({
      initialState: {
        systemPrompt: prompt.prompt,
        model,
        thinkingLevel: run.thinkingLevel ?? "medium",
        tools,
        messages: boundedContext(prior),
      },
      sessionId: run.runId,
      steeringMode: "one-at-a-time",
      followUpMode: "one-at-a-time",
      toolExecution: "parallel",
      streamFn: interruptibleStream,
      getApiKey: (provider) =>
        provider === run.provider.kind
          ? run.provider.apiKey ?? (run.provider.kind === "ollama" ? "ollama" : undefined)
          : undefined,
      transformContext: async (messages) => boundedContext(messages),
      beforeToolCall: async ({ toolCall, args }) => {
        const definition = definitionByName.get(toolCall.name);
        if (!definition) return { block: true, reason: "backend_tool_not_registered" };
        const local = this.policyEngine.check(run, definition);
        if (!local.allowed) return { block: true, reason: local.reason ?? "policy_denied" };
        const canonicalArguments = toJsonObject(args);
        const digest = argumentsDigest(toolCall.name, canonicalArguments);
        const decision = await this.options.osClient.verifyCapability({
          runId: run.runId,
          userId: run.scope.userId,
          ...(run.scope.projectId ? { projectId: run.scope.projectId } : {}),
          toolCallId: toolCall.id,
          toolName: toolCall.name,
          argumentsDigest: digest,
          risk: definition.risk,
          runCapabilityToken: run.runCapabilityToken,
          arguments: canonicalArguments,
        });
        executionState.decisions.set(toolCall.id, decision);
        if (!decision.allowed && decision.reason === "approval_required") {
          const approvalScope = requiredApprovalForTool(toolCall.name);
          if (
            !decision.approvalId || !decision.expiresAt ||
            !approvalScope || decision.approvalScope !== approvalScope ||
            !approvalDeclared(run, toolCall.name, approvalScope)
          ) {
            throw new HarnessError("approval_contract_invalid", "OS returned an invalid approval contract.");
          }
          executionState.pendingApproval = {
            id: decision.approvalId,
            toolCallId: toolCall.id,
            toolName: toolCall.name,
            argumentsDigest: digest,
            risk: definition.risk,
            approvalScope,
            expiresAt: decision.expiresAt,
          };
          mapper.interruptTool(toolCall.id);
        }
        return decision.allowed ? undefined : { block: true, reason: decision.reason ?? "approval_required" };
      },
      afterToolCall: async ({ result }) => ({
        content: result.content.map((content) =>
          content.type === "text" ? { type: "text" as const, text: String(sanitizeUntrusted(content.text)) } : content,
        ),
        details: sanitizeUntrusted(result.details),
      }),
    });

    const unsubscribe = agent.subscribe(async (event) => {
      for (const mapped of mapper.map(event)) await emit(mapped);
    });
    const timeoutMs = Math.min(Math.max(run.timeoutMs ?? 120_000, 1_000), 600_000);
    let timedOut = false;
    const timer = setTimeout(() => {
      timedOut = true;
      agent.abort();
    }, timeoutMs);
    try {
      if (resumeApproval) {
        const approved = validateApprovedResume(run, resumeApproval, definitions);
        executionState.decisions.set(resumeApproval.toolCallId, {
          allowed: true,
          executionToken: approved.executionToken,
        });
        for (const event of mapper.toolStarted(
          resumeApproval.toolCallId,
          resumeApproval.toolName,
          approved.arguments,
        )) await emit(event);
        const resumedTool = this.createTool(run, approved.definition, executionState);
        const toolResult = await resumedTool.execute(resumeApproval.toolCallId, approved.arguments);
        const details = sanitizeUntrusted(toolResult.details) as JsonObject;
        const replayResult = details.content ?? details;
        for (const event of mapper.toolFinished(
          resumeApproval.toolCallId,
          sanitizeUntrusted(replayResult),
        )) await emit(event);
        const toolResultMessage: ToolResultMessage = {
          role: "toolResult",
          toolCallId: resumeApproval.toolCallId,
          toolName: resumeApproval.toolName,
          content: toolResult.content,
          details,
          isError: false,
          timestamp: Date.now(),
        };
        agent.state.messages = boundedContext([
          ...prior,
          resumedAssistantMessage(run, resumeApproval, approved.arguments),
          toolResultMessage,
        ]);
      }
      await this.providerFetchScope.run(
        { kind: run.provider.kind, baseUrl: run.provider.baseUrl },
        () => resumeApproval ? agent.continue() : agent.prompt(messageText(current!.content)),
      );
      if (executionState.pendingApproval) {
        const pending = executionState.pendingApproval;
        const interrupt = sanitizeUntrusted({
          id: pending.id,
          reason: "tool_call",
          message: `Approve ${pending.toolName} (${pending.approvalScope}) for this project?`,
          toolCallId: pending.toolCallId,
          expiresAt: pending.expiresAt,
          responseSchema: {
            type: "object",
            properties: { approved: { const: true } },
            required: ["approved"],
            additionalProperties: false,
          },
          metadata: {
            originalRunId: run.runId,
            projectId: run.scope.projectId ?? null,
            toolName: pending.toolName,
            toolCallId: pending.toolCallId,
            argumentsDigest: pending.argumentsDigest,
            risk: pending.risk,
            approvalScope: pending.approvalScope,
          },
        });
        await emit(mapper.interrupted([interrupt]));
        return {
          runId: run.runId,
          content: "",
          evidence: [...executionState.evidence.values()],
          validationReports: executionState.validationReports,
          stopReason: "interrupted",
        };
      }
      const terminal = finalAssistant(agent.state.messages);
      if (terminal?.stopReason === "error") {
        throw new HarnessError(
          "provider_error",
          redactText(terminal.errorMessage ?? "The provider failed.", [run.provider.apiKey, run.runCapabilityToken]),
        );
      }
      if (terminal?.stopReason === "aborted") {
        throw new HarnessError(timedOut ? "agent_timeout" : "agent_aborted", timedOut ? "The agent run timed out." : "The agent run was aborted.");
      }
      if (terminal?.stopReason === "length") {
        throw new HarnessError("provider_output_truncated", "The provider output reached its token limit and is incomplete.");
      }
      const content = finalAssistantText(agent.state.messages);
      if (!content) throw new HarnessError("provider_empty_response", "The provider returned no final answer.");
      const evidence = [...executionState.evidence.values()];
      if (executionState.validatedEpoch < executionState.scientificEpoch) {
        executionState.validationReports.push({
          status: "fail",
          findings: [{
            code: "analysis_validation_missing",
            severity: "error",
            message: "Scientific computation completed without a subsequent deterministic analysis.validate call.",
            path: "validationReports",
          }],
          checkedAt: new Date().toISOString(),
          validatorVersion: "shennong-run-validator/v1",
        });
      }
      const citationReport = answerValidation(content, evidence, executionState.scientificEpoch > 0);
      if (citationReport) executionState.validationReports.push(citationReport);
      const validationFailed = executionState.validationReports.some(({ status }) => status === "fail");
      const result: RunResult = {
        runId: run.runId,
        content,
        evidence,
        validationReports: executionState.validationReports,
        stopReason: validationFailed ? "validation_failed" : "stop",
      };
      if (validationFailed) {
        await emit(mapper.error(
          "analysis_validation_failed",
          "The biomedical answer failed deterministic validation and was not accepted as a completed result.",
        ));
      } else {
        await emit(mapper.finished(sanitizeUntrusted({ stopReason: result.stopReason, evidenceCount: evidence.length })));
      }
      await this.options.osClient.finishRun(run.runId, result);
      return result;
    } catch (error) {
      const code = error instanceof HarnessError ? error.code : "agent_run_failed";
      const message = redactText(error instanceof Error ? error.message : "Agent run failed.", [
        run.provider.apiKey,
        run.runCapabilityToken,
      ]);
      try {
        await emit(mapper.error(code, message));
      } finally {
        await this.options.osClient.finishRun(run.runId, undefined, { code, message });
      }
      throw error;
    } finally {
      clearTimeout(timer);
      unsubscribe();
      agent.reset();
    }
  }

  metadata(run: RunRequest): PersistedRunMetadata {
    const prompt = this.promptCompiler.compile(run);
    return {
      runId: run.runId,
      ...(run.parentRunId ? { parentRunId: run.parentRunId } : {}),
      userId: run.scope.userId,
      threadId: run.scope.threadId,
      ...(run.scope.projectId ? { projectId: run.scope.projectId } : {}),
      model: run.provider.model,
      provider: run.provider.kind,
      promptDigest: prompt.digest,
      platformPolicyVersion: prompt.platformPolicyVersion,
      biomedicalPolicyVersion: prompt.biomedicalPolicyVersion,
      skills: (run.context?.selectedSkills ?? []).map(({ id, version, digest }) => ({ id, version, digest })),
    };
  }

  private validateRun(run: RunRequest): void {
    if (
      !run.runId || run.runId.length > 128 ||
      !run.scope?.threadId || run.scope.threadId.length > 128 ||
      !run.scope.userId || run.scope.userId.length > 128 ||
      (run.scope.projectId !== undefined && (!run.scope.projectId || run.scope.projectId.length > 128))
    ) throw new HarnessError("run_identity_invalid", "Run, thread, user, and project IDs must be bounded.");
    if (!run.runCapabilityToken || run.runCapabilityToken.length < 32 || run.runCapabilityToken.length > 4096) {
      throw new HarnessError("run_capability_invalid", "A bounded short-lived run capability is required.");
    }
    if (!run.provider?.model || !run.provider.baseUrl) throw new HarnessError("provider_unconfigured", "No model provider is configured for this run.");
    if (
      !["openai", "deepseek", "ollama", "llama-cpp", "openai-compatible"].includes(run.provider.kind) ||
      run.provider.model.length > 256 ||
      run.provider.baseUrl.length > 2048 ||
      (run.provider.apiKey !== undefined &&
        (typeof run.provider.apiKey !== "string" || run.provider.apiKey.length > 65_536)) ||
      !["public_only", "allow_private"].includes(run.provider.dataPolicy) ||
      (run.provider.capabilities !== undefined && (
        !Array.isArray(run.provider.capabilities) ||
        run.provider.capabilities.some((capability) => !["tools", "thinking", "images"].includes(capability))
      ))
    ) throw new HarnessError("provider_invalid", "The authorized provider configuration is invalid.");
    if (
      !["user", "admin"].includes(run.scope.role) ||
      !["global-read", "project-analysis", "project-write"].includes(run.toolProfile)
    ) throw new HarnessError("run_scope_invalid", "The authorized role or tool profile is invalid.");
    if (run.scope.providerDataPolicy !== run.provider.dataPolicy) throw new HarnessError("provider_policy_mismatch", "Provider data policy does not match the authorized run scope.");
    if (
      !Array.isArray(run.messages) ||
      !run.messages.length ||
      run.messages.length > 200 ||
      run.messages.some((message) =>
        !message ||
        !["user", "assistant"].includes(message.role) ||
        !(typeof message.content === "string" || Array.isArray(message.content)))
    ) throw new HarnessError("messages_invalid", "A run supports 1..200 canonical user or assistant messages.");
    if (run.timeoutMs !== undefined && (!Number.isFinite(run.timeoutMs) || run.timeoutMs <= 0)) {
      throw new HarnessError("run_timeout_invalid", "Run timeout must be a positive finite number.");
    }
    const resume = run.resumeApproval;
    if (resume && (
      !run.parentRunId || run.parentRunId !== resume.originalRunId ||
      !resume.interruptId || resume.interruptId.length > 128 ||
      !resume.toolCallId || resume.toolCallId.length > 256 ||
      !resume.toolName || resume.toolName.length > 128 ||
      !/^[a-f0-9]{64}$/.test(resume.argumentsDigest) ||
      !["resolved", "cancelled"].includes(resume.status) ||
      !["runtime.compute", "project.write", "runtime.cancel", "artifact.register"].includes(resume.approvalScope) ||
      Number.isNaN(new Date(resume.expiresAt).getTime())
    )) {
      throw new HarnessError("approval_resume_invalid", "The persisted approval continuation is malformed.");
    }
    const selectedSkills = run.context?.selectedSkills;
    if (selectedSkills !== undefined && !Array.isArray(selectedSkills)) {
      throw new HarnessError("skill_selection_invalid", "Selected Skills must be an array.");
    }
    for (const skill of selectedSkills ?? []) {
      if (
        !skill.id || !skill.name || !skill.version ||
        !/^sha256:[a-f0-9]{64}$/.test(skill.digest) ||
        (skill.loadRef !== undefined && (typeof skill.loadRef !== "string" || skill.loadRef.length > 256)) ||
        (skill.content !== undefined && (typeof skill.content !== "string" || skill.content.length > 65_536)) ||
        !skill.permissions || !Array.isArray(skill.permissions.tools)
      ) throw new HarnessError("skill_selection_invalid", "A selected Skill is missing trusted version, digest, or permissions.");
    }
  }
}
