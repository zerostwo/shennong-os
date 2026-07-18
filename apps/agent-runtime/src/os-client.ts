import type {
  AgUiEvent,
  BackendToolExecutionRequest,
  BackendToolExecutionResult,
  CapabilityDecision,
  CapabilityVerificationRequest,
  PersistedRunMetadata,
  RunRequest,
  RunResult,
  JsonValue,
} from "./types.js";

const MAX_CALLBACK_BYTES = 2 * 1024 * 1024;

export interface AgentRunIdentity {
  threadId: string;
  runId: string;
  parentRunId?: string;
}

export interface AgentUserMessageInput extends AgentRunIdentity {
  message: {
    id?: string;
    role: "user";
    content: string | JsonValue[];
  };
}

export interface OsInternalClient {
  bootstrap(input: AgentRunIdentity): Promise<RunRequest>;
  persistLatestUserMessage(input: AgentUserMessageInput): Promise<void>;
  recordRunMetadata(metadata: PersistedRunMetadata): Promise<void>;
  appendEvent(runId: string, event: AgUiEvent): Promise<string | undefined>;
  finishRun(runId: string, result: RunResult | undefined, error?: { code: string; message: string }): Promise<void>;
  verifyCapability(request: CapabilityVerificationRequest): Promise<CapabilityDecision>;
  executeTool(request: BackendToolExecutionRequest): Promise<BackendToolExecutionResult>;
}

function callbackBase(raw: string): URL {
  const url = new URL(raw);
  if (!["http:", "https:"].includes(url.protocol) || url.username || url.password || url.search || url.hash) {
    throw new Error("os_internal_url_invalid");
  }
  return new URL(`${url.origin}${url.pathname.replace(/\/+$/, "")}/`);
}

async function boundedJson(response: Response): Promise<unknown> {
  const reader = response.body?.getReader();
  if (!reader) return null;
  const chunks: Uint8Array[] = [];
  let size = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    size += value.byteLength;
    if (size > MAX_CALLBACK_BYTES) {
      await reader.cancel();
      throw new Error("os_callback_too_large");
    }
    chunks.push(value);
  }
  const merged = Buffer.concat(chunks.map((chunk) => Buffer.from(chunk)));
  return merged.length ? JSON.parse(merged.toString("utf8")) : null;
}

export class HttpOsInternalClient implements OsInternalClient {
  private readonly base: URL;

  constructor(
    baseUrl: string,
    private readonly serviceToken: string,
    private readonly fetchImpl: typeof fetch = globalThis.fetch.bind(globalThis),
  ) {
    this.base = callbackBase(baseUrl);
    if (serviceToken.length < 32) throw new Error("os_service_token_invalid");
  }

  private async request(path: string, value: unknown): Promise<unknown> {
    const target = new URL(path.replace(/^\//, ""), this.base);
    if (target.origin !== this.base.origin || !target.pathname.startsWith(this.base.pathname)) {
      throw new Error("os_callback_path_invalid");
    }
    const response = await this.fetchImpl(target, {
      method: "POST",
      redirect: "error",
      headers: {
        authorization: `Bearer ${this.serviceToken}`,
        "content-type": "application/json",
        "x-shennong-service": "agent-runtime",
      },
      body: JSON.stringify(value),
    });
    const payload = await boundedJson(response);
    if (!response.ok) {
      const code =
        payload && typeof payload === "object" && "error" in payload
          ? String((payload as { error: unknown }).error)
          : `os_callback_${response.status}`;
      throw new Error(code);
    }
    if (payload && typeof payload === "object" && "data" in payload) {
      return (payload as { data: unknown }).data;
    }
    return payload;
  }

  async bootstrap(input: AgentRunIdentity): Promise<RunRequest> {
    return (await this.request("api/v1/agent/runs", {
      thread_id: input.threadId,
      run_id: input.runId,
      parent_run_id: input.parentRunId,
    })) as RunRequest;
  }

  async persistLatestUserMessage(input: AgentUserMessageInput): Promise<void> {
    const { message } = input;
    await this.request(`api/v1/threads/${encodeURIComponent(input.threadId)}/messages`, {
      id: message.id,
      run_id: input.runId,
      role: "user",
      content: message.content,
    });
  }

  async recordRunMetadata(metadata: PersistedRunMetadata): Promise<void> {
    await this.request(`api/v1/agent/runs/${encodeURIComponent(metadata.runId)}/metadata`, metadata);
  }

  async appendEvent(runId: string, event: AgUiEvent): Promise<string | undefined> {
    const data = (await this.request(`api/v1/agent/runs/${encodeURIComponent(runId)}/events`, event)) as
      | { cursor?: string }
      | undefined;
    return data?.cursor;
  }

  async finishRun(
    runId: string,
    result: RunResult | undefined,
    error?: { code: string; message: string },
  ): Promise<void> {
    const status = error
      ? "failed"
      : result?.stopReason === "approval_rejected"
        ? "cancelled"
      : result?.stopReason === "validation_failed"
        ? "failed_validation"
        : "succeeded";
    await this.request(`api/v1/agent/runs/${encodeURIComponent(runId)}/finish`, {
      status,
      result,
      error,
    });
  }

  async verifyCapability(request: CapabilityVerificationRequest): Promise<CapabilityDecision> {
    return (await this.request(
      `api/v1/agent/runs/${encodeURIComponent(request.runId)}/approvals/verify`,
      request,
    )) as CapabilityDecision;
  }

  async executeTool(request: BackendToolExecutionRequest): Promise<BackendToolExecutionResult> {
    return (await this.request(
      `api/v1/agent/runs/${encodeURIComponent(request.runId)}/tools`,
      request,
    )) as BackendToolExecutionResult;
  }
}
