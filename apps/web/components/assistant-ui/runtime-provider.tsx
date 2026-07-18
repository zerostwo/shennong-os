"use client";

import {
  AssistantRuntimeProvider,
  CompositeAttachmentAdapter,
  ExportedMessageRepository,
  SimpleImageAttachmentAdapter,
  SimpleTextAttachmentAdapter,
  type ExportedMessageRepositoryItem,
  type ThreadHistoryAdapter,
} from "@assistant-ui/react";
import { HttpAgent } from "@ag-ui/client";
import { useAgUiRuntime } from "@assistant-ui/react-ag-ui";
import { usePathname, useRouter } from "next/navigation";
import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState } from "react";
import {
  deleteOsThread,
  listOsThreads,
  loadOsThread,
  persistAssistantMessage,
  updateOsThread,
  type OsThread,
} from "@/lib/assistant-runtime";
import { resumeOsRun } from "@/lib/assistant-run-resume";
import { randomUuid } from "@/lib/random-uuid";
import { listAiProviders, type AiProviderRecord } from "@/lib/api/adapter";

export type ThinkingLevel = "off" | "low" | "medium" | "high" | "xhigh";

type RuntimeContextValue = {
  activeThreadId: string;
  hasPersistedThread: boolean;
  projectId?: string;
  providers: AiProviderRecord[];
  providerId: string;
  setProviderId: (providerId: string) => Promise<void>;
  thinkingLevel: ThinkingLevel;
  setThinkingLevel: (level: ThinkingLevel) => void;
  refreshThreads: () => Promise<void>;
};

const RuntimeContext = createContext<RuntimeContextValue | null>(null);

function runtimeErrorMessage(reason: unknown, fallback: string) {
  if (reason instanceof Error && typeof reason.message === "string" && reason.message !== "[object Object]") return reason.message;
  if (reason && typeof reason === "object" && "message" in reason && typeof reason.message === "string" && reason.message !== "[object Object]") return reason.message;
  return fallback;
}
class ShennongHttpAgent extends HttpAgent {
  override runAgent(...[parameters, subscriber]: Parameters<HttpAgent["runAgent"]>) {
    return super.runAgent({ ...parameters, runId: randomUuid() }, subscriber);
  }
}

export function useShennongAssistantRuntime() {
  return useContext(RuntimeContext);
}

function threadPath(threadId: string, projectId?: string) {
  return projectId
    ? `/projects/${encodeURIComponent(projectId)}/chat/${encodeURIComponent(threadId)}`
    : `/chat/${encodeURIComponent(threadId)}`;
}

export function ShennongRuntimeProvider({
  initialThreadId,
  projectId,
  children,
}: {
  initialThreadId?: string;
  projectId?: string;
  children: React.ReactNode;
}) {
  const router = useRouter();
  const pathname = usePathname();
  const [selectedThreadId, setSelectedThreadId] = useState(initialThreadId);
  const [draftGeneration, setDraftGeneration] = useState(0);
  const [threads, setThreads] = useState<OsThread[]>([]);
  const [isLoadingThreads, setIsLoadingThreads] = useState(true);
  const [runtimeError, setRuntimeError] = useState("");
  const [providers, setProviders] = useState<AiProviderRecord[]>([]);
  const [providerId, setProviderIdState] = useState("");
  const [thinkingLevel, setThinkingLevel] = useState<ThinkingLevel>("medium");
  const resumableRun = useRef<{ threadId: string; runId: string } | null>(null);

  useEffect(() => setSelectedThreadId(initialThreadId), [initialThreadId]);

  useEffect(() => {
    let cancelled = false;
    void listAiProviders().then((items) => {
      if (cancelled) return;
      const enabled = items.filter((item) => item.enabled);
      setProviders(enabled);
      setProviderIdState((current) => current || enabled.find((item) => item.isDefault)?.id || enabled[0]?.id || "");
    }).catch((error) => setRuntimeError(runtimeErrorMessage(error, "Models could not be loaded")));
    return () => { cancelled = true; };
  }, []);

  const agent = useMemo(() => {
    // A new unsaved draft needs a fresh AG-UI agent even though it has no thread id yet.
    void draftGeneration;
    return new ShennongHttpAgent({
      url: "/api/agent",
      threadId: selectedThreadId,
      headers: {
        "x-shennong-ui": "assistant-ui",
        ...(projectId ? { "x-shennong-project-id": projectId } : {}),
        ...(providerId ? { "x-shennong-provider-id": providerId } : {}),
        "x-shennong-thinking-level": thinkingLevel,
      },
    });
  }, [draftGeneration, projectId, providerId, selectedThreadId, thinkingLevel]);
  const activeThreadId = agent.threadId;

  const refreshThreads = useCallback(async () => {
    setIsLoadingThreads(true);
    try {
      setThreads(await listOsThreads(projectId));
    } catch (error) {
      setRuntimeError(runtimeErrorMessage(error, "Conversations could not be loaded"));
    } finally {
      setIsLoadingThreads(false);
    }
  }, [projectId]);

  useEffect(() => { void refreshThreads(); }, [refreshThreads]);
  useEffect(() => {
    const selected = threads.find((thread) => thread.id === activeThreadId);
    if (selected?.providerId) setProviderIdState(selected.providerId);
  }, [activeThreadId, threads]);
  useEffect(() => {
    const refresh = () => void refreshThreads();
    window.addEventListener("shennong:threads-updated", refresh);
    return () => window.removeEventListener("shennong:threads-updated", refresh);
  }, [refreshThreads]);

  const loadThread = useCallback(async (threadId: string) => {
    const loaded = await loadOsThread(threadId);
    resumableRun.current = loaded.activeRunId
      ? { threadId, runId: loaded.activeRunId }
      : null;
    return loaded;
  }, []);

  const history = useMemo<ThreadHistoryAdapter>(() => ({
    async load() {
      const loaded = await loadThread(activeThreadId);
      return {
        ...ExportedMessageRepository.fromArray(loaded.messages),
        ...(loaded.state !== undefined ? { state: loaded.state as never } : {}),
        ...(loaded.running ? { unstable_resume: true } : {}),
      };
    },
    async append(item: ExportedMessageRepositoryItem) {
      const message = item.message as unknown as Record<string, unknown>;
      // Browser history may persist the user's optimistic entry before a run.
      // Assistant/tool messages are authoritative OS outputs (finish callback
      // or durable event replay) and must never be written with user authority.
      if (message.role === "user") {
        await persistAssistantMessage(activeThreadId, message).catch((error) => {
          if ((error as { status?: number }).status !== 409) throw error;
        });
      }
      if (!initialThreadId && pathname === "/") router.replace(threadPath(activeThreadId, projectId));
      window.setTimeout(() => void refreshThreads(), 350);
    },
    async *resume(options) {
      const run = resumableRun.current;
      if (!run || run.threadId !== activeThreadId) return;
      try {
        yield* resumeOsRun({
          runId: run.runId,
          abortSignal: options.abortSignal,
        });
      } finally {
        if (resumableRun.current?.runId === run.runId) resumableRun.current = null;
        window.setTimeout(() => void refreshThreads(), 350);
      }
    },
  }), [activeThreadId, initialThreadId, loadThread, pathname, projectId, refreshThreads, router]);

  const threadList = useMemo(() => ({
    threadId: activeThreadId,
    isLoading: isLoadingThreads,
    threads: threads.filter((thread) => thread.status === "regular").map((thread) => ({
      id: thread.id,
      remoteId: thread.id,
      status: "regular" as const,
      title: thread.title,
      custom: { projectId: thread.projectId, updatedAt: thread.updatedAt },
    })),
    archivedThreads: threads.filter((thread) => thread.status === "archived").map((thread) => ({
      id: thread.id,
      remoteId: thread.id,
      status: "archived" as const,
      title: thread.title,
      custom: { projectId: thread.projectId, updatedAt: thread.updatedAt },
    })),
    async onSwitchToNewThread() {
      setSelectedThreadId(undefined);
      setDraftGeneration((value) => value + 1);
      router.push(projectId ? `/projects/${encodeURIComponent(projectId)}/chat` : "/");
    },
    async onSwitchToThread(threadId: string) {
      const loaded = await loadThread(threadId);
      setSelectedThreadId(threadId);
      router.push(threadPath(threadId, projectId));
      return { messages: loaded.messages, state: loaded.state as never, unstable_resume: loaded.running };
    },
    async onRename(threadId: string, title: string) {
      await updateOsThread(threadId, { title });
      await refreshThreads();
    },
    async onArchive(threadId: string) {
      await updateOsThread(threadId, { status: "archived" });
      await refreshThreads();
    },
    async onUnarchive(threadId: string) {
      await updateOsThread(threadId, { status: "regular" });
      await refreshThreads();
    },
    async onDelete(threadId: string) {
      await deleteOsThread(threadId);
      await refreshThreads();
      if (threadId === activeThreadId) router.push(projectId ? `/projects/${encodeURIComponent(projectId)}/chat` : "/");
    },
  }), [activeThreadId, isLoadingThreads, loadThread, projectId, refreshThreads, router, threads]);

  const runtime = useAgUiRuntime({
    agent,
    showThinking: true,
    autoCancelPendingToolCalls: true,
    onError: (error) => setRuntimeError(runtimeErrorMessage(error, "The Agent request failed")),
    adapters: {
      history,
      threadList,
      attachments: new CompositeAttachmentAdapter([
        new SimpleImageAttachmentAdapter(),
        new SimpleTextAttachmentAdapter(),
      ]),
    },
  });

  const setProviderId = useCallback(async (nextProviderId: string) => {
    setProviderIdState(nextProviderId);
    if (initialThreadId) {
      await updateOsThread(activeThreadId, { provider_id: nextProviderId });
      await refreshThreads();
    }
  }, [activeThreadId, initialThreadId, refreshThreads]);

  return (
    <RuntimeContext.Provider value={{ activeThreadId, hasPersistedThread: Boolean(initialThreadId), projectId, providers, providerId, setProviderId, thinkingLevel, setThinkingLevel, refreshThreads }}>
      <AssistantRuntimeProvider runtime={runtime}>
        {runtimeError ? <div className="runtime-error-banner" role="alert">{runtimeError}<button onClick={() => setRuntimeError("")} aria-label="Dismiss">×</button></div> : null}
        {children}
      </AssistantRuntimeProvider>
    </RuntimeContext.Provider>
  );
}
