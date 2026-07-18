import type {
  ChatModelRunResult,
  ThreadAssistantMessagePart,
  ToolCallMessagePart,
} from "@assistant-ui/react";
import type { AgUiInterrupt } from "@assistant-ui/react-ag-ui";

type JsonRecord = Record<string, unknown>;

type ReplayText = {
  kind: "text";
  key: string;
};

type ReplayTool = {
  kind: "tool";
  key: string;
};

type ReplayPart = ReplayText | ReplayTool;

type ToolState = {
  id: string;
  name: string;
  argsText: string;
  args: Record<string, unknown>;
  result?: unknown;
  isError?: boolean;
};

type SseFrame = {
  cursor: string;
  payload: JsonRecord;
};

export type ResumeOsRunOptions = {
  runId: string;
  abortSignal: AbortSignal;
  fetchImpl?: typeof fetch;
  retryDelayMs?: number;
};

const TERMINAL_EVENTS = new Set(["RUN_FINISHED", "RUN_ERROR", "RUN_CANCELLED"]);

function asRecord(value: unknown): JsonRecord | null {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? value as JsonRecord
    : null;
}

function stringValue(value: unknown): string | undefined {
  return typeof value === "string" ? value : undefined;
}

function optionalRecord(value: unknown, field: string): JsonRecord | undefined {
  if (value === undefined) return undefined;
  const parsed = asRecord(value);
  if (!parsed) throw new Error(`Invalid AG-UI ${field}`);
  return parsed;
}

function parseInterrupts(event: JsonRecord): AgUiInterrupt[] | undefined {
  const outcome = asRecord(event.outcome);
  if (!outcome || outcome.type === undefined || outcome.type === "success") return undefined;
  if (outcome.type !== "interrupt" || !Array.isArray(outcome.interrupts) || outcome.interrupts.length === 0) {
    throw new Error("Invalid AG-UI interrupt outcome");
  }
  return outcome.interrupts.map((value) => {
    const interrupt = asRecord(value);
    const id = interrupt && stringValue(interrupt.id);
    const reason = interrupt && stringValue(interrupt.reason);
    if (!interrupt || !id || id.length > 128 || !reason || reason.length > 128) {
      throw new Error("Invalid AG-UI interrupt");
    }
    const message = stringValue(interrupt.message);
    const toolCallId = stringValue(interrupt.toolCallId);
    const expiresAt = stringValue(interrupt.expiresAt);
    if (
      (interrupt.message !== undefined && message === undefined) ||
      (interrupt.toolCallId !== undefined && (!toolCallId || toolCallId.length > 256)) ||
      (interrupt.expiresAt !== undefined && (!expiresAt || Number.isNaN(new Date(expiresAt).getTime())))
    ) {
      throw new Error("Invalid AG-UI interrupt");
    }
    return {
      id,
      reason,
      ...(message !== undefined ? { message } : {}),
      ...(toolCallId !== undefined ? { toolCallId } : {}),
      ...(expiresAt !== undefined ? { expiresAt } : {}),
      ...(interrupt.responseSchema !== undefined
        ? { responseSchema: optionalRecord(interrupt.responseSchema, "interrupt response schema") }
        : {}),
      ...(interrupt.metadata !== undefined
        ? { metadata: optionalRecord(interrupt.metadata, "interrupt metadata") }
        : {}),
    };
  });
}

function parseToolResult(value: unknown): unknown {
  if (typeof value !== "string") return value;
  try {
    return JSON.parse(value);
  } catch {
    return value;
  }
}

function parseCursor(value: string): bigint {
  if (!/^[1-9][0-9]*$/.test(value)) throw new Error("Invalid AG-UI replay cursor");
  return BigInt(value);
}

function nextFrame(buffer: string): { frame: string; rest: string } | null {
  const boundary = /\r?\n\r?\n/.exec(buffer);
  if (!boundary || boundary.index === undefined) return null;
  return {
    frame: buffer.slice(0, boundary.index),
    rest: buffer.slice(boundary.index + boundary[0].length),
  };
}

function parseFrame(frame: string): SseFrame | null {
  let cursor = "";
  const data: string[] = [];
  for (const line of frame.split(/\r?\n/)) {
    if (!line || line.startsWith(":")) continue;
    const separator = line.indexOf(":");
    const field = separator === -1 ? line : line.slice(0, separator);
    let value = separator === -1 ? "" : line.slice(separator + 1);
    if (value.startsWith(" ")) value = value.slice(1);
    if (field === "id") cursor = value;
    if (field === "data") data.push(value);
  }
  if (!cursor && data.length === 0) return null;
  parseCursor(cursor);
  const payload = asRecord(JSON.parse(data.join("\n")));
  if (!payload || typeof payload.type !== "string") {
    throw new Error("Invalid AG-UI replay event");
  }
  return { cursor, payload };
}

async function* readSseFrames(
  body: ReadableStream<Uint8Array>,
  abortSignal: AbortSignal,
): AsyncGenerator<SseFrame, void, unknown> {
  const reader = body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  try {
    while (!abortSignal.aborted) {
      const { done, value } = await reader.read();
      if (done) break;
      buffer += decoder.decode(value, { stream: true });
      for (;;) {
        const parsed = nextFrame(buffer);
        if (!parsed) break;
        buffer = parsed.rest;
        const frame = parseFrame(parsed.frame);
        if (frame) yield frame;
      }
    }
  } finally {
    reader.releaseLock();
  }
}

async function retryDelay(milliseconds: number, abortSignal: AbortSignal): Promise<void> {
  if (milliseconds <= 0 || abortSignal.aborted) return;
  await new Promise<void>((resolve) => {
    const complete = () => {
      clearTimeout(timer);
      abortSignal.removeEventListener("abort", complete);
      resolve();
    };
    const timer = window.setTimeout(complete, milliseconds);
    abortSignal.addEventListener("abort", complete, { once: true });
  });
}

class AgUiReplayReducer {
  private status: ChatModelRunResult["status"] = { type: "running" };
  private interrupts: AgUiInterrupt[] | undefined;
  private readonly order: ReplayPart[] = [];
  private readonly texts = new Map<string, { text: string; touched: boolean }>();
  private readonly tools = new Map<string, ToolState>();
  private activeTextId: string | undefined;
  private generatedText = 0;

  apply(event: JsonRecord): ChatModelRunResult | null {
    const type = stringValue(event.type);
    if (!type) throw new Error("Invalid AG-UI replay event");
    switch (type) {
      case "RUN_STARTED":
        this.reset();
        this.status = { type: "running" };
        return this.snapshot();
      case "RUN_FINISHED": {
        this.interrupts = parseInterrupts(event);
        this.status = this.interrupts
          ? { type: "requires-action", reason: "interrupt" }
          : { type: "complete", reason: "unknown" };
        return this.snapshot();
      }
      case "RUN_ERROR":
        this.interrupts = undefined;
        this.status = {
          type: "incomplete",
          reason: "error",
          ...(stringValue(event.message) ? { error: stringValue(event.message) } : {}),
        };
        return this.snapshot();
      case "RUN_CANCELLED":
        this.interrupts = undefined;
        this.status = { type: "incomplete", reason: "cancelled" };
        return this.snapshot();
      case "TEXT_MESSAGE_START": {
        const key = stringValue(event.messageId) ?? this.textKey();
        this.ensureText(key).touched = true;
        this.activeTextId = key;
        return this.snapshot();
      }
      case "TEXT_MESSAGE_CONTENT":
      case "TEXT_MESSAGE_CHUNK": {
        const delta = stringValue(event.delta);
        if (!delta) return null;
        const key = stringValue(event.messageId) ?? this.activeTextId ?? this.textKey();
        const text = this.ensureText(key);
        text.text += delta;
        text.touched = true;
        this.activeTextId = key;
        return this.snapshot();
      }
      case "TEXT_MESSAGE_END":
        if (!event.messageId || event.messageId === this.activeTextId) this.activeTextId = undefined;
        return this.snapshot();
      case "TOOL_CALL_START": {
        const id = stringValue(event.toolCallId);
        if (!id) throw new Error("Invalid AG-UI tool replay event");
        this.activeTextId = undefined;
        if (!this.tools.has(id)) {
          this.tools.set(id, {
            id,
            name: stringValue(event.toolCallName) ?? "tool",
            argsText: "",
            args: {},
          });
          this.order.push({ kind: "tool", key: id });
        }
        return this.snapshot();
      }
      case "TOOL_CALL_ARGS":
      case "TOOL_CALL_CHUNK": {
        const id = stringValue(event.toolCallId);
        const delta = stringValue(event.delta) ?? "";
        const tool = id ? this.tools.get(id) : undefined;
        if (!tool) return null;
        tool.argsText += delta;
        try {
          const args = asRecord(JSON.parse(tool.argsText));
          tool.args = args ?? {};
        } catch {
          tool.args = {};
        }
        return this.snapshot();
      }
      case "TOOL_CALL_RESULT": {
        const id = stringValue(event.toolCallId);
        if (!id) throw new Error("Invalid AG-UI tool result replay event");
        let tool = this.tools.get(id);
        if (!tool) {
          tool = { id, name: "tool", argsText: "", args: {} };
          this.tools.set(id, tool);
          this.order.push({ kind: "tool", key: id });
        }
        tool.result = parseToolResult(event.content);
        if (typeof event.isError === "boolean") tool.isError = event.isError;
        return this.snapshot();
      }
      case "TOOL_CALL_END":
        return this.snapshot();
      default:
        return null;
    }
  }

  private reset() {
    this.order.length = 0;
    this.texts.clear();
    this.tools.clear();
    this.activeTextId = undefined;
    this.generatedText = 0;
    this.interrupts = undefined;
  }

  private textKey() {
    this.generatedText += 1;
    return `resume-text-${this.generatedText}`;
  }

  private ensureText(key: string) {
    let text = this.texts.get(key);
    if (!text) {
      text = { text: "", touched: false };
      this.texts.set(key, text);
      this.order.push({ kind: "text", key });
    }
    return text;
  }

  private snapshot(): ChatModelRunResult {
    const content: ThreadAssistantMessagePart[] = [];
    for (const item of this.order) {
      if (item.kind === "text") {
        const text = this.texts.get(item.key);
        if (text?.touched) content.push({ type: "text", text: text.text });
        continue;
      }
      const tool = this.tools.get(item.key);
      if (!tool) continue;
      content.push({
        type: "tool-call",
        toolCallId: tool.id,
        toolName: tool.name,
        argsText: tool.argsText,
        args: tool.args,
        ...(tool.result !== undefined ? { result: tool.result } : {}),
        ...(tool.isError !== undefined ? { isError: tool.isError } : {}),
      } as ToolCallMessagePart);
    }
    return {
      content,
      status: this.status,
      ...(this.interrupts
        ? { metadata: { custom: { agui: { interrupts: this.interrupts } } } }
        : {}),
    };
  }
}

export async function* resumeOsRun({
  runId,
  abortSignal,
  fetchImpl = globalThis.fetch.bind(globalThis),
  retryDelayMs = 250,
}: ResumeOsRunOptions): AsyncGenerator<ChatModelRunResult, void, unknown> {
  const reducer = new AgUiReplayReducer();
  let cursor = "0";
  while (!abortSignal.aborted) {
    const query = new URLSearchParams({ after: cursor, limit: "500" });
    const headers = new Headers({ accept: "text/event-stream" });
    if (cursor !== "0") headers.set("Last-Event-ID", cursor);
    try {
      const response = await fetchImpl(
        `/api/v1/runs/${encodeURIComponent(runId)}/events/stream?${query}`,
        {
          method: "GET",
          credentials: "include",
          cache: "no-store",
          headers,
          signal: abortSignal,
        },
      );
      if (!response.ok || !response.body) {
        throw new Error(`Run replay failed (${response.status})`);
      }
      for await (const frame of readSseFrames(response.body, abortSignal)) {
        if (parseCursor(frame.cursor) <= BigInt(cursor)) continue;
        cursor = frame.cursor;
        const update = reducer.apply(frame.payload);
        if (update) yield update;
        if (TERMINAL_EVENTS.has(String(frame.payload.type))) return;
      }
    } catch (error) {
      if (abortSignal.aborted) return;
      if (error instanceof Error && /^Run replay failed \((?:4\d\d)\)$/.test(error.message)) {
        throw error;
      }
    }
    await retryDelay(retryDelayMs, abortSignal);
  }
}
