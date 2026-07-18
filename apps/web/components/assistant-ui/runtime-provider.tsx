"use client";

import {
  AssistantRuntimeProvider,
  ExportedMessageRepository,
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

type RuntimeContextValue = {
  activeThreadId: string;
  hasPersistedThread: boolean;
  projectId?: string;
  refreshThreads: () => Promise<void>;
};

const RuntimeContext = createContext<RuntimeContextValue | null>(null);

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
  const resumableRun = useRef<{ threadId: string; runId: string } | null>(null);

  useEffect(() => setSelectedThreadId(initialThreadId), [initialThreadId]);

  const agent = useMemo(() => {
    // A new unsaved draft needs a fresh AG-UI agent even though it has no thread id yet.
    void draftGeneration;
    return new ShennongHttpAgent({
      url: "/api/agent",
      threadId: selectedThreadId,
      headers: {
        "x-shennong-ui": "assistant-ui",
        ...(projectId ? { "x-shennong-project-id": projectId } : {}),
      },
    });
  }, [draftGeneration, projectId, selectedThreadId]);
  const activeThreadId = agent.threadId;

  const refreshThreads = useCallback(async () => {
    setIsLoadingThreads(true);
    try {
      setThreads(await listOsThreads(projectId));
    } catch (error) {
      setRuntimeError(error instanceof Error ? error.message : "Conversations could not be loaded");
    } finally {
      setIsLoadingThreads(false);
    }
  }, [projectId]);

  useEffect(() => { void refreshThreads(); }, [refreshThreads]);
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
    showThinking: false,
    autoCancelPendingToolCalls: true,
    onError: (error) => setRuntimeError(error.message),
    adapters: { history, threadList },
  });

  return (
    <RuntimeContext.Provider value={{ activeThreadId, hasPersistedThread: Boolean(initialThreadId), projectId, refreshThreads }}>
      <AssistantRuntimeProvider runtime={runtime}>
        {runtimeError ? <div className="runtime-error-banner" role="alert">{runtimeError}<button onClick={() => setRuntimeError("")} aria-label="Dismiss">×</button></div> : null}
        {children}
      </AssistantRuntimeProvider>
    </RuntimeContext.Provider>
  );
}
