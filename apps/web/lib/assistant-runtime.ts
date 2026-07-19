import { fromThreadMessageLike, type MessageStatus, type ThreadMessage } from "@assistant-ui/react";
import { fromAgUiMessages } from "@assistant-ui/react-ag-ui";
import { randomUuid } from "./random-uuid";

export type OsThread = {
  id: string;
  title: string;
  status: "regular" | "archived";
  projectId?: string;
  providerId?: string;
  updatedAt?: string;
};

type JsonRecord = Record<string, unknown>;

const API = "/api/v1";

function record(value: unknown): JsonRecord {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? value as JsonRecord
    : {};
}

function rows(value: unknown, key: string): JsonRecord[] {
  const root = record(value);
  const candidate = "data" in root ? root.data : value;
  if (Array.isArray(candidate)) return candidate.map(record);
  const nested = record(candidate);
  const list = nested[key] ?? nested.items;
  return Array.isArray(list) ? list.map(record) : [];
}

async function request(path: string, init?: RequestInit): Promise<unknown> {
  const response = await fetch(`${API}${path}`, {
    ...init,
    credentials: "include",
    headers: {
      accept: "application/json",
      ...(init?.body ? { "content-type": "application/json" } : {}),
      ...(init?.headers ?? {}),
    },
  });
  const payload = await response.json().catch(() => ({}));
  if (!response.ok) {
    const body = record(payload);
    const nested = record(body.error);
    const message = typeof nested.message === "string"
      ? nested.message
      : typeof body.message === "string"
        ? body.message
        : typeof body.error === "string"
          ? body.error
          : `Request failed (${response.status})`;
    const error = new Error(message);
    Object.assign(error, {
      status: response.status,
      code: typeof nested.code === "string" ? nested.code : body.code,
      requestId: nested.request_id ?? body.request_id,
    });
    throw error;
  }
  const body = record(payload);
  return "data" in body ? body.data : payload;
}

export async function listOsThreads(projectId?: string): Promise<OsThread[]> {
  const query = new URLSearchParams();
  if (projectId) query.set("project_id", projectId);
  else query.set("scope", "personal");
  const value = await request(`/threads${query.size ? `?${query}` : ""}`);
  return rows(value, "threads").map((item) => ({
    id: String(item.id ?? ""),
    title: String(item.title ?? "New chat"),
    status: item.status === "archived" ? "archived" as const : "regular" as const,
    projectId: typeof item.project_id === "string" ? item.project_id : undefined,
    providerId: typeof item.provider_id === "string" ? item.provider_id : undefined,
    updatedAt: typeof item.updated_at === "string" ? item.updated_at : undefined,
  })).filter((item) => item.id.length > 0);
}

export async function updateOsThread(id: string, value: JsonRecord): Promise<void> {
  await request(`/threads/${encodeURIComponent(id)}`, {
    method: "PATCH",
    body: JSON.stringify(value),
  });
}

export async function deleteOsThread(id: string): Promise<void> {
  await request(`/threads/${encodeURIComponent(id)}`, { method: "DELETE" });
}

function contentText(value: unknown): string {
  if (typeof value === "string") return value;
  if (Array.isArray(value)) {
    return value.map((part) => {
      const item = record(part);
      return typeof item.text === "string" ? item.text : typeof item.content === "string" ? item.content : "";
    }).join("");
  }
  const item = record(value);
  return typeof item.text === "string"
    ? item.text
    : typeof item.content === "string"
      ? item.content
      : typeof item.markdown === "string"
        ? item.markdown
        : "";
}

function normalizeAgUiMessage(item: JsonRecord, index: number): JsonRecord | null {
  const embedded = record(item.agui_message ?? item.ag_ui_message);
  if (Object.keys(embedded).length > 0) return embedded;
  const role = String(item.role ?? "").toLowerCase();
  if (role !== "user" && role !== "assistant" && role !== "system" && role !== "tool") return null;
  const contentJson = item.content_json ?? item.content;
  const normalized: JsonRecord = {
    id: String(item.id ?? `persisted-${index}`),
    role,
    content: contentText(contentJson),
  };
  if (role === "tool" && typeof item.tool_call_id === "string") normalized.toolCallId = item.tool_call_id;
  const metadata = record(item.metadata);
  if (Object.keys(metadata).length > 0) normalized.metadata = metadata;
  return normalized;
}

export type LoadedOsThread = {
  messages: readonly ThreadMessage[];
  state?: unknown;
  running: boolean;
  activeRunId?: string;
  lastEventCursor?: string;
};

export async function loadOsThread(id: string): Promise<LoadedOsThread> {
  try {
    const encodedId = encodeURIComponent(id);
    const [value, activeValue] = await Promise.all([
      request(`/threads/${encodedId}/messages`),
      request(`/threads/${encodedId}/runs/active`),
    ]);
    const root = record(value);
    const activeRoot = record(activeValue);
    const activeRun = record(activeRoot.run);
    const activeRunId = typeof activeRun.id === "string" ? activeRun.id : undefined;
    const messageRows = rows(value, "messages");
    const agUiMessages = messageRows.flatMap((item, index) => {
      const normalized = normalizeAgUiMessage(item, index);
      return normalized ? [normalized] : [];
    });
    const messageLikes = fromAgUiMessages(agUiMessages as never, { showThinking: true });
    const fallbackStatus: MessageStatus = { type: "complete", reason: "unknown" };
    return {
      messages: messageLikes.map((message, index) => fromThreadMessageLike(message, message.id ?? `persisted-${index}`, message.status ?? fallbackStatus)),
      state: root.state,
      running: Boolean(activeRunId),
      activeRunId,
      lastEventCursor: typeof activeRoot.last_event_cursor === "string"
        ? activeRoot.last_event_cursor
        : undefined,
    };
  } catch (error) {
    if (record(error).status === 404) return { messages: [], running: false };
    throw error;
  }
}

export async function persistAssistantMessage(threadId: string, message: JsonRecord): Promise<void> {
  await request(`/threads/${encodeURIComponent(threadId)}/messages`, {
    method: "POST",
    headers: { "Idempotency-Key": String(message.id ?? randomUuid()) },
    body: JSON.stringify({
      id: message.id,
      role: message.role,
      content_json: message.content,
      metadata: message.metadata ?? {},
      source: "assistant-ui-history",
    }),
  });
}
