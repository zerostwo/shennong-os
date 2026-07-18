import { EventSchemas, type BaseEvent } from "@ag-ui/core";
import { EventEncoder } from "@ag-ui/encoder";
import type { AgentEvent } from "@earendil-works/pi-agent-core";
import { sanitizeUntrusted } from "./security.js";
import type { AgUiEvent, JsonValue } from "./types.js";

function now(): number {
  return Date.now();
}

export class AgUiEventMapper {
  private activeMessageId?: string;
  private lastAssistantMessageId?: string;
  private activeMessageText = "";
  private stepIndex = 0;
  private readonly interruptedToolCalls = new Set<string>();

  constructor(
    private readonly threadId: string,
    private readonly runId: string,
  ) {}

  started(): AgUiEvent {
    return { type: "RUN_STARTED", threadId: this.threadId, runId: this.runId, timestamp: now() };
  }

  finished(result?: JsonValue): AgUiEvent {
    return { type: "RUN_FINISHED", threadId: this.threadId, runId: this.runId, result, timestamp: now() };
  }

  interrupted(interrupts: JsonValue[]): AgUiEvent {
    return {
      type: "RUN_FINISHED",
      threadId: this.threadId,
      runId: this.runId,
      outcome: { type: "interrupt", interrupts },
      timestamp: now(),
    };
  }

  interruptTool(toolCallId: string): void {
    this.interruptedToolCalls.add(toolCallId);
  }

  toolStarted(toolCallId: string, toolName: string, args: JsonValue): AgUiEvent[] {
    return [
      {
        type: "TOOL_CALL_START",
        toolCallId,
        toolCallName: toolName,
        parentMessageId: this.lastAssistantMessageId,
        timestamp: now(),
      },
      {
        type: "TOOL_CALL_ARGS",
        toolCallId,
        delta: JSON.stringify(sanitizeUntrusted(args)),
        timestamp: now(),
      },
    ];
  }

  toolFinished(toolCallId: string, result: JsonValue, isError = false): AgUiEvent[] {
    return [
      { type: "TOOL_CALL_END", toolCallId, timestamp: now() },
      {
        type: "TOOL_CALL_RESULT",
        messageId: `${toolCallId}:result`,
        toolCallId,
        content: JSON.stringify(sanitizeUntrusted(result)),
        role: "tool",
        isError,
        timestamp: now(),
      },
    ];
  }

  error(code: string, message: string): AgUiEvent {
    return { type: "RUN_ERROR", threadId: this.threadId, runId: this.runId, code, message, timestamp: now() };
  }

  map(event: AgentEvent): AgUiEvent[] {
    switch (event.type) {
      case "turn_start": {
        this.stepIndex += 1;
        return [{ type: "STEP_STARTED", stepName: `turn-${this.stepIndex}`, timestamp: now() }];
      }
      case "turn_end":
        return [{ type: "STEP_FINISHED", stepName: `turn-${this.stepIndex}`, timestamp: now() }];
      case "message_start": {
        if (event.message.role !== "assistant") return [];
        this.activeMessageId = `${this.runId}:assistant:${this.stepIndex}`;
        this.lastAssistantMessageId = this.activeMessageId;
        this.activeMessageText = "";
        return [
          {
            type: "TEXT_MESSAGE_START",
            messageId: this.activeMessageId,
            role: "assistant",
            timestamp: now(),
          },
        ];
      }
      case "message_update": {
        if (event.assistantMessageEvent.type !== "text_delta" || !this.activeMessageId) return [];
        this.activeMessageText += event.assistantMessageEvent.delta;
        return [
          {
            type: "TEXT_MESSAGE_CONTENT",
            messageId: this.activeMessageId,
            delta: event.assistantMessageEvent.delta,
            timestamp: now(),
          },
        ];
      }
      case "message_end": {
        if (event.message.role !== "assistant" || !this.activeMessageId) return [];
        const messageId = this.activeMessageId;
        const finalText = event.message.content
          .filter((content): content is Extract<typeof content, { type: "text" }> => content.type === "text")
          .map(({ text }) => text)
          .join("");
        const remainder = finalText.startsWith(this.activeMessageText)
          ? finalText.slice(this.activeMessageText.length)
          : "";
        delete this.activeMessageId;
        this.activeMessageText = "";
        return [
          ...(remainder ? [{ type: "TEXT_MESSAGE_CONTENT", messageId, delta: remainder, timestamp: now() }] : []),
          { type: "TEXT_MESSAGE_END", messageId, timestamp: now() },
        ];
      }
      case "tool_execution_start":
        return this.toolStarted(event.toolCallId, event.toolName, sanitizeUntrusted(event.args));
      case "tool_execution_update":
        return [
          {
            type: "ACTIVITY_SNAPSHOT",
            messageId: `${event.toolCallId}:activity`,
            activityType: "tool-progress",
            content: sanitizeUntrusted(event.partialResult),
            replace: true,
            timestamp: now(),
          },
        ];
      case "tool_execution_end":
        if (this.interruptedToolCalls.delete(event.toolCallId)) return [];
        return this.toolFinished(event.toolCallId, sanitizeUntrusted(event.result), event.isError);
      default:
        return [];
    }
  }
}

const sseEncoder = new EventEncoder({ accept: "text/event-stream" });

export function encodeSse(event: AgUiEvent, cursor?: string): string {
  const validated = EventSchemas.parse(event) as BaseEvent;
  const encoded = sseEncoder.encodeSSE(validated);
  return cursor ? `id: ${cursor}\n${encoded}` : encoded;
}
