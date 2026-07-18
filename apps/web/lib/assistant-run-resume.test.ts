import { describe, expect, it, vi } from "vitest";

import { resumeOsRun } from "./assistant-run-resume";

function sse(...events: Array<{ cursor: string; payload: Record<string, unknown> }>) {
  return events
    .map(({ cursor, payload }) => `id: ${cursor}\ndata: ${JSON.stringify(payload)}\n\n`)
    .join("");
}

describe("durable AG-UI run resume", () => {
  it("reconnects after the last cursor and applies every event exactly once through RUN_FINISHED", async () => {
    const runId = "00000000-0000-4000-8000-000000000042";
    const fetchImpl = vi.fn<typeof fetch>()
      .mockResolvedValueOnce(new Response(sse(
        { cursor: "1", payload: { type: "RUN_STARTED", runId } },
        { cursor: "2", payload: { type: "TEXT_MESSAGE_START", messageId: "assistant-1" } },
        { cursor: "3", payload: { type: "TEXT_MESSAGE_CONTENT", messageId: "assistant-1", delta: "hello" } },
      ), { headers: { "content-type": "text/event-stream" } }))
      .mockResolvedValueOnce(new Response(sse(
        // A proxy may replay the acknowledged frame at the reconnect boundary.
        { cursor: "3", payload: { type: "TEXT_MESSAGE_CONTENT", messageId: "assistant-1", delta: "hello" } },
        { cursor: "4", payload: { type: "TEXT_MESSAGE_CONTENT", messageId: "assistant-1", delta: " world" } },
        { cursor: "5", payload: { type: "TEXT_MESSAGE_END", messageId: "assistant-1" } },
        { cursor: "6", payload: { type: "RUN_FINISHED", runId } },
      ), { headers: { "content-type": "text/event-stream" } }));
    const controller = new AbortController();
    const updates = [];

    for await (const update of resumeOsRun({
      runId,
      abortSignal: controller.signal,
      fetchImpl,
      retryDelayMs: 0,
    })) {
      updates.push(update);
    }

    expect(fetchImpl).toHaveBeenCalledTimes(2);
    expect(String(fetchImpl.mock.calls[0]?.[0])).toContain(`/runs/${runId}/events/stream?after=0`);
    expect(String(fetchImpl.mock.calls[1]?.[0])).toContain(`/runs/${runId}/events/stream?after=3`);
    const secondHeaders = new Headers(fetchImpl.mock.calls[1]?.[1]?.headers);
    expect(secondHeaders.get("last-event-id")).toBe("3");
    expect(fetchImpl.mock.calls.every(([, init]) => init?.method === "GET")).toBe(true);

    const final = updates.at(-1);
    const text = final?.content?.find((part) => part.type === "text");
    expect(text).toEqual({ type: "text", text: "hello world" });
    expect(final?.status).toEqual({ type: "complete", reason: "unknown" });
  });

  it("rehydrates native assistant-ui interrupts from the durable RUN_FINISHED outcome", async () => {
    const runId = "00000000-0000-4000-8000-000000000042";
    const expiresAt = new Date(Date.now() + 60_000).toISOString();
    const interrupt = {
      id: "00000000-0000-4000-8000-000000000043",
      reason: "tool_call",
      message: "Approve project.write_file?",
      toolCallId: "tool-write-1",
      expiresAt,
      responseSchema: {
        type: "object",
        properties: { approved: { const: true } },
        required: ["approved"],
        additionalProperties: false,
      },
      metadata: {
        originalRunId: runId,
        argumentsDigest: "a".repeat(64),
        approvalScope: "project.write",
      },
    };
    const fetchImpl = vi.fn<typeof fetch>().mockResolvedValue(new Response(sse(
      { cursor: "1", payload: { type: "RUN_STARTED", runId } },
      { cursor: "2", payload: { type: "TOOL_CALL_START", toolCallId: "tool-write-1", toolCallName: "project.write_file" } },
      { cursor: "3", payload: { type: "TOOL_CALL_ARGS", toolCallId: "tool-write-1", delta: "{\"uri\":\"project://current/README.md\"}" } },
      {
        cursor: "4",
        payload: { type: "RUN_FINISHED", runId, outcome: { type: "interrupt", interrupts: [interrupt] } },
      },
    ), { headers: { "content-type": "text/event-stream" } }));
    const updates = [];

    for await (const update of resumeOsRun({
      runId,
      abortSignal: new AbortController().signal,
      fetchImpl,
      retryDelayMs: 0,
    })) {
      updates.push(update);
    }

    const final = updates.at(-1);
    expect(final?.status).toEqual({ type: "requires-action", reason: "interrupt" });
    expect(final?.metadata?.custom).toEqual({ agui: { interrupts: [interrupt] } });
    expect(final?.content?.find((part) => part.type === "tool-call")).toMatchObject({
      toolCallId: "tool-write-1",
      toolName: "project.write_file",
      args: { uri: "project://current/README.md" },
    });
  });
});
