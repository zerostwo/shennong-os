import { readFileSync } from "node:fs";
import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import { AgUiEventMapper, encodeSse } from "./ag-ui.js";
import { HarnessError, ProviderFetchScope, ShennongAgentHarness } from "./harness.js";
import { HttpOsInternalClient, type OsInternalClient } from "./os-client.js";
import { timingSafeSecret } from "./security.js";
import type { AgUiRunAgentInput, ConversationMessage, RunRequest } from "./types.js";

const MAX_REQUEST_BYTES = 1024 * 1024;
const MAX_CONTENT_BYTES = 512 * 1024;

async function readJson(request: IncomingMessage): Promise<unknown> {
  const chunks: Buffer[] = [];
  let bytes = 0;
  for await (const chunk of request) {
    const value = Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk);
    bytes += value.length;
    if (bytes > MAX_REQUEST_BYTES) throw new HarnessError("request_too_large", "RunAgentInput exceeds 1 MiB.");
    chunks.push(value);
  }
  try {
    return JSON.parse(Buffer.concat(chunks).toString("utf8"));
  } catch {
    throw new HarnessError("invalid_json", "Request body is not valid JSON.");
  }
}

function boundedId(value: unknown): value is string {
  return typeof value === "string" && value.length >= 1 && value.length <= 128 && /^[A-Za-z0-9._:-]+$/.test(value);
}

function validateRunAgentInput(value: unknown): AgUiRunAgentInput {
  if (!value || typeof value !== "object") throw new HarnessError("invalid_run_input", "RunAgentInput must be an object.");
  const input = value as Partial<AgUiRunAgentInput>;
  if (!boundedId(input.threadId) || !boundedId(input.runId) || !Array.isArray(input.messages) || input.messages.length < 1 || input.messages.length > 200) {
    throw new HarnessError("invalid_run_input", "threadId, runId, and 1..200 messages are required.");
  }
  let contentBytes = 0;
  for (const message of input.messages) {
    if (
      !message ||
      typeof message !== "object" ||
      typeof message.role !== "string" ||
      message.content === null ||
      !(typeof message.content === "string" || Array.isArray(message.content))
    ) {
      throw new HarnessError("invalid_run_input", "Every AG-UI message needs a role and content.");
    }
    contentBytes += Buffer.byteLength(JSON.stringify(message.content));
  }
  if (contentBytes > MAX_CONTENT_BYTES) throw new HarnessError("request_too_large", "Message content exceeds 512 KiB.");
  if (input.parentRunId !== undefined && !boundedId(input.parentRunId)) {
    throw new HarnessError("invalid_run_input", "parentRunId is invalid.");
  }
  if (input.resume !== undefined) {
    if (!input.parentRunId || !Array.isArray(input.resume) || input.resume.length !== 1) {
      throw new HarnessError("invalid_run_input", "A continuation needs one resume response and parentRunId.");
    }
    const entry = input.resume[0];
    if (!entry || !boundedId(entry.interruptId) || !["resolved", "cancelled"].includes(entry.status)) {
      throw new HarnessError("invalid_run_input", "The AG-UI resume response is invalid.");
    }
    if (entry.status === "resolved") {
      const payload = entry.payload;
      if (
        !payload || typeof payload !== "object" || Array.isArray(payload) ||
        Object.keys(payload).length !== 1 || (payload as { approved?: unknown }).approved !== true
      ) throw new HarnessError("invalid_run_input", "Resolved approvals require the exact {approved:true} payload.");
    } else if (entry.payload !== undefined && entry.payload !== null) {
      throw new HarnessError("invalid_run_input", "Cancelled approvals cannot include a payload.");
    }
  }
  return input as AgUiRunAgentInput;
}

function canonicalConversation(messages: RunRequest["messages"]): ConversationMessage[] {
  return messages
    .filter((message) => message.role === "user" || message.role === "assistant")
    .map((message) => ({
      ...(message.id ? { id: message.id } : {}),
      role: message.role,
      content: message.content,
      ...(message.timestamp ? { timestamp: message.timestamp } : {}),
    }));
}

function validateResumeContract(input: AgUiRunAgentInput, authorized: RunRequest): void {
  const response = input.resume?.[0];
  const resume = authorized.resumeApproval;
  if (!response && !resume) return;
  if (
    !response || !resume ||
    input.parentRunId !== authorized.parentRunId ||
    resume.originalRunId !== input.parentRunId ||
    response.interruptId !== resume.interruptId ||
    response.status !== resume.status
  ) {
    throw new HarnessError("approval_resume_mismatch", "OS approval continuation does not match the native AG-UI response.");
  }
}

function json(response: ServerResponse, status: number, value: unknown): void {
  const body = JSON.stringify(value);
  response.writeHead(status, {
    "content-type": "application/json",
    "content-length": Buffer.byteLength(body),
    "cache-control": "no-store",
  });
  response.end(body);
}

function sse(response: ServerResponse): void {
  response.writeHead(200, {
    "content-type": "text/event-stream; charset=utf-8",
    "cache-control": "no-cache, no-store",
    connection: "keep-alive",
    "x-accel-buffering": "no",
  });
  response.flushHeaders();
}

function errorCode(error: unknown): string {
  if (error instanceof HarnessError) return error.code;
  if (error instanceof Error && /^[a-z0-9_:-]{1,128}$/.test(error.message)) return error.message;
  return "agent_runtime_failed";
}

export interface AgentRuntimeServerOptions {
  secret: string;
  osClient: OsInternalClient;
  providerFetchScope?: ProviderFetchScope;
}

export function secretFromEnv(name: string, environment: NodeJS.ProcessEnv = process.env): string {
  const direct = environment[name];
  const file = environment[`${name}_FILE`];
  if (direct !== undefined && file !== undefined) throw new Error(`${name.toLowerCase()}_ambiguous`);
  const value = file ? readFileSync(file, "utf8").replace(/\r?\n$/, "") : (direct ?? "");
  if (value.includes("\0") || Buffer.byteLength(value) > 16_384) throw new Error(`${name.toLowerCase()}_invalid`);
  return value;
}

export function createAgentRuntimeServer(options: AgentRuntimeServerOptions) {
  if (options.secret.length < 32) throw new Error("agent_runtime_secret_invalid");
  const fetchScope = options.providerFetchScope ?? new ProviderFetchScope();
  const harness = new ShennongAgentHarness({ osClient: options.osClient, providerFetchScope: fetchScope });
  return createServer(async (request, response) => {
    if (request.method === "GET" && request.url === "/health") {
      json(response, 200, { status: "ok", runtime: "pi-agent-core", version: "0.80.10" });
      return;
    }
    if (request.method !== "POST" || !["/v1/agent", "/api/agent"].includes(request.url ?? "")) {
      json(response, 404, { error: { code: "not_found" } });
      return;
    }
    if (!timingSafeSecret(request.headers.authorization, options.secret)) {
      json(response, 401, { error: { code: "unauthorized" } });
      return;
    }

    let input: AgUiRunAgentInput;
    try {
      input = validateRunAgentInput(await readJson(request));
    } catch (error) {
      json(response, errorCode(error) === "request_too_large" ? 413 : 422, {
        error: { code: errorCode(error), message: error instanceof Error ? error.message : "Invalid request." },
      });
      return;
    }

    sse(response);
    const directMapper = new AgUiEventMapper(input.threadId, input.runId);
    let harnessErrorEmitted = false;
    try {
      // Client-supplied tools, state, context, and forwardedProps are deliberately ignored.
      if (!input.resume) {
        const latestUser = [...input.messages].reverse().find(({ role }) => role === "user");
        if (!latestUser) throw new HarnessError("user_message_missing", "RunAgentInput requires a user message.");
        await options.osClient.persistLatestUserMessage({
          threadId: input.threadId,
          runId: input.runId,
          ...(input.parentRunId ? { parentRunId: input.parentRunId } : {}),
          message: {
            ...(latestUser.id ? { id: latestUser.id } : {}),
            role: "user",
            content: latestUser.content,
          },
        });
      }
      const authorized = await options.osClient.bootstrap({
        threadId: input.threadId,
        runId: input.runId,
        ...(input.parentRunId ? { parentRunId: input.parentRunId } : {}),
      });
      if (authorized.runId !== input.runId || authorized.scope.threadId !== input.threadId) {
        throw new HarnessError("run_scope_mismatch", "OS callback returned a different run or thread scope.");
      }
      validateResumeContract(input, authorized);
      authorized.messages = canonicalConversation(authorized.messages);
      await harness.run(authorized, (event, cursor) => {
        if (event.type === "RUN_ERROR") harnessErrorEmitted = true;
        if (!response.writableEnded && !response.destroyed) response.write(encodeSse(event, cursor));
      });
    } catch (error) {
      const code = errorCode(error);
      const message = error instanceof Error ? error.message : "Agent runtime failed.";
      // Bootstrap and preflight failures have not passed through the harness event sink.
      if (!harnessErrorEmitted && !response.writableEnded && !response.destroyed) {
        response.write(encodeSse(directMapper.error(code, message)));
      }
    } finally {
      if (!response.writableEnded) response.end();
    }
  });
}

export function serverFromEnv(): ReturnType<typeof createAgentRuntimeServer> {
  const secret = secretFromEnv("SHENNONG_AGENT_RUNTIME_SECRET");
  const serviceToken = secretFromEnv("SHENNONG_OS_SERVICE_TOKEN");
  const internalUrl = process.env.SHENNONG_OS_INTERNAL_URL ?? "";
  const originalFetch = globalThis.fetch.bind(globalThis);
  const osClient = new HttpOsInternalClient(internalUrl, serviceToken, originalFetch);
  const providerFetchScope = new ProviderFetchScope();
  providerFetchScope.install();
  return createAgentRuntimeServer({ secret, osClient, providerFetchScope });
}
