import { render, waitFor } from "@testing-library/react";
import type { ThreadHistoryAdapter } from "@assistant-ui/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { ShennongRuntimeProvider } from "./runtime-provider";

const mocks = vi.hoisted(() => ({
  httpAgent: vi.fn(function MockHttpAgent(this: { threadId: string }, options: { threadId?: string }) {
    this.threadId = options.threadId ?? "draft-thread";
  }),
  httpAgentRun: vi.fn(),
  listOsThreads: vi.fn(async () => []),
  persistAssistantMessage: vi.fn(async () => undefined),
  loadOsThread: vi.fn(async () => ({
    messages: [],
    running: true,
    activeRunId: "00000000-0000-4000-8000-000000000042",
  })),
  resumeOsRun: vi.fn(async function* () {
    yield { content: [{ type: "text", text: "resumed" }], status: { type: "complete", reason: "unknown" } };
  }),
  runtimeOptions: null as { agent?: { runAgent: (parameters: Record<string, unknown>, subscriber: unknown) => unknown }; adapters?: { history?: ThreadHistoryAdapter } } | null,
}));

vi.mock("@ag-ui/client", () => {
  mocks.httpAgent.prototype.runAgent = mocks.httpAgentRun;
  return { HttpAgent: mocks.httpAgent };
});
vi.mock("@assistant-ui/react", () => ({
  AssistantRuntimeProvider: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  ExportedMessageRepository: { fromArray: vi.fn(() => ({})) },
  CompositeAttachmentAdapter: class { constructor(adapters: unknown[]) { void adapters; } },
  SimpleImageAttachmentAdapter: class {},
  SimpleTextAttachmentAdapter: class {},
}));
vi.mock("@assistant-ui/react-ag-ui", () => ({
  useAgUiRuntime: vi.fn((options: { agent?: { runAgent: (parameters: Record<string, unknown>, subscriber: unknown) => unknown }; adapters?: { history?: ThreadHistoryAdapter } }) => {
    mocks.runtimeOptions = options;
    return {};
  }),
}));
vi.mock("next/navigation", () => ({
  usePathname: () => "/projects/project-1/chat/thread-1",
  useRouter: () => ({ push: vi.fn(), replace: vi.fn() }),
}));
vi.mock("@/lib/assistant-runtime", () => ({
  deleteOsThread: vi.fn(),
  listOsThreads: mocks.listOsThreads,
  loadOsThread: mocks.loadOsThread,
  persistAssistantMessage: mocks.persistAssistantMessage,
  updateOsThread: vi.fn(),
}));
vi.mock("@/lib/assistant-run-resume", () => ({ resumeOsRun: mocks.resumeOsRun }));
vi.mock("@/lib/api/adapter", () => ({
  listAiProviders: vi.fn(async () => [{ id: "provider-1", name: "Local", model: "qwen", enabled: true, isDefault: true }]),
}));

describe("ShennongRuntimeProvider Project transport", () => {
  beforeEach(() => {
    mocks.httpAgent.mockClear();
    mocks.httpAgentRun.mockClear();
    mocks.listOsThreads.mockClear();
    mocks.loadOsThread.mockClear();
    mocks.persistAssistantMessage.mockClear();
    mocks.resumeOsRun.mockClear();
    mocks.runtimeOptions = null;
  });

  it("replaces every caller-supplied run id with a UUID", async () => {
    render(
      <ShennongRuntimeProvider initialThreadId="thread-1" projectId="project-1">
        <div>Child</div>
      </ShennongRuntimeProvider>,
    );
    await waitFor(() => expect(mocks.runtimeOptions?.agent).toBeDefined());
    mocks.runtimeOptions!.agent!.runAgent({ runId: "run-1", threadId: "thread-1" }, {});
    expect(mocks.httpAgentRun).toHaveBeenCalledWith(
      expect.objectContaining({ runId: expect.stringMatching(/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i) }),
      {},
    );
  });

  it("passes the explicit Project id to the AG-UI request headers", async () => {
    render(
      <ShennongRuntimeProvider initialThreadId="thread-1" projectId="project-1">
        <div>Child</div>
      </ShennongRuntimeProvider>,
    );

    await waitFor(() => expect(mocks.httpAgent).toHaveBeenCalledWith(expect.objectContaining({
      headers: expect.objectContaining({ "x-shennong-provider-id": "provider-1" }),
    })));
    expect(mocks.httpAgent).toHaveBeenCalledWith(expect.objectContaining({
      url: "/api/agent",
      threadId: "thread-1",
      headers: expect.objectContaining({
        "x-shennong-project-id": "project-1",
        "x-shennong-provider-id": "provider-1",
        "x-shennong-thinking-level": "medium",
      }),
    }));
  });

  it("provides assistant-ui history.resume without starting a second AG-UI run", async () => {
    render(
      <ShennongRuntimeProvider initialThreadId="thread-1" projectId="project-1">
        <div>Child</div>
      </ShennongRuntimeProvider>,
    );

    await waitFor(() => expect(mocks.runtimeOptions?.adapters?.history).toBeDefined());
    const history = mocks.runtimeOptions?.adapters?.history;
    expect(history?.resume).toBeTypeOf("function");
    expect(await history?.load()).toMatchObject({ unstable_resume: true });

    const controller = new AbortController();
    const agentCallsBeforeResume = mocks.httpAgent.mock.calls.length;
    const updates = [];
    for await (const update of history!.resume!({ abortSignal: controller.signal } as never)) {
      updates.push(update);
    }

    expect(updates).toHaveLength(1);
    expect(mocks.resumeOsRun).toHaveBeenCalledWith(expect.objectContaining({
      runId: "00000000-0000-4000-8000-000000000042",
      abortSignal: controller.signal,
    }));
    expect(mocks.httpAgent).toHaveBeenCalledTimes(agentCallsBeforeResume);
  });

  it("persists optimistic user history but leaves assistant interrupt output OS-authoritative", async () => {
    render(
      <ShennongRuntimeProvider initialThreadId="thread-1" projectId="project-1">
        <div>Child</div>
      </ShennongRuntimeProvider>,
    );

    await waitFor(() => expect(mocks.runtimeOptions?.adapters?.history).toBeDefined());
    const history = mocks.runtimeOptions!.adapters!.history!;
    await history.append({
      parentId: null,
      message: { id: "user-1", role: "user", content: [{ type: "text", text: "Analyze" }] },
    } as never);
    await history.append({
      parentId: "user-1",
      message: {
        id: "assistant-interrupt",
        role: "assistant",
        content: [],
        status: { type: "requires-action", reason: "interrupt" },
      },
    } as never);

    expect(mocks.persistAssistantMessage).toHaveBeenCalledTimes(1);
    expect(mocks.persistAssistantMessage).toHaveBeenCalledWith(
      "thread-1",
      expect.objectContaining({ id: "user-1", role: "user" }),
    );
  });
});
