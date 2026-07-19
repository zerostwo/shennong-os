"use client";

import Link from "next/link";
import { Brain, ChevronDown, FolderKanban, LogIn, Settings2 } from "lucide-react";
import { useEffect, useState } from "react";
import { AppShell } from "@/components/app-shell";
import { ShennongRuntimeProvider, useShennongAssistantRuntime, type ThinkingLevel } from "@/components/assistant-ui/runtime-provider";
import { ShennongThread, ThreadSkillSelector } from "@/components/assistant-ui/thread";
import { getSession } from "@/lib/api/adapter";

const thinkingLabels: Record<ThinkingLevel, string> = {
  off: "Thinking off",
  low: "Low reasoning",
  medium: "Medium reasoning",
  high: "High reasoning",
  xhigh: "Extra high reasoning",
};

function ChatControls() {
  const runtime = useShennongAssistantRuntime();
  if (!runtime) return null;
  return (
    <div className="chat-runtime-controls">
      <label className="chat-header-select chat-model-select">
        <span className="sr-only">Model</span>
        <select value={runtime.providerId} onChange={(event) => void runtime.setProviderId(event.target.value)} aria-label="Model">
          {!runtime.providers.length ? <option value="">No model configured</option> : null}
          {runtime.providers.map((provider) => <option key={provider.id} value={provider.id}>{provider.name} · {provider.model}</option>)}
        </select>
        <ChevronDown />
      </label>
      <label className="chat-header-select chat-thinking-select" title="Reasoning intensity">
        <Brain />
        <select value={runtime.thinkingLevel} onChange={(event) => runtime.setThinkingLevel(event.target.value as ThinkingLevel)} aria-label="Reasoning intensity">
          {(Object.keys(thinkingLabels) as ThinkingLevel[]).map((level) => <option key={level} value={level}>{thinkingLabels[level]}</option>)}
        </select>
        <ChevronDown />
      </label>
    </div>
  );
}
export function ChatView({ threadId, projectId, initialPrompt }: { threadId?: string; projectId?: string; initialPrompt?: string }) {
  const [authenticated, setAuthenticated] = useState<boolean | null>(null);
  const [handoffPrompt, setHandoffPrompt] = useState(initialPrompt);
  useEffect(() => {
    let cancelled = false;
    void getSession()
      .then((session) => { if (!cancelled) setAuthenticated(session.authenticated); })
      .catch(() => { if (!cancelled) setAuthenticated(false); });
    return () => { cancelled = true; };
  }, []);
  useEffect(() => {
    if (!projectId || initialPrompt) return;
    const key = `shennong.project-handoff:${projectId}`;
    const prompt = window.sessionStorage.getItem(key);
    if (!prompt) return;
    window.sessionStorage.removeItem(key);
    setHandoffPrompt(prompt);
  }, [initialPrompt, projectId]);

  if (authenticated !== true) {
    return (
      <AppShell active="chat">
        <div className="chat-workspace guest-chat-workspace" aria-busy={authenticated === null}>
          <header className="chat-header"><strong>Shennong Agent</strong></header>
          <main className="guest-chat-main">
            {authenticated === null ? <div className="chat-auth-loading" aria-live="polite">Preparing your workspace…</div> : (
              <section className="guest-chat-card">
                <span className="guest-chat-icon"><LogIn /></span>
                <h1>Sign in to chat with Shennong</h1>
                <p>Explore public Resources and documentation as a guest. Sign in when you are ready to start a private research conversation.</p>
                <Link className="guest-chat-action" href="/auth/sign-in?returnTo=%2F"><LogIn />Sign in to start a chat</Link>
              </section>
            )}
          </main>
        </div>
      </AppShell>
    );
  }
  return (
    <ShennongRuntimeProvider initialThreadId={threadId} projectId={projectId}>
      <AppShell active="chat" assistantThreads>
        <div className="chat-workspace has-assistant-ui">
          <header className="chat-header">
            <ChatControls />
            <ThreadSkillSelector />
            <button className="chat-settings-button" aria-label="Manage models" onClick={() => window.dispatchEvent(new CustomEvent("shennong:open-settings", { detail: "models" }))}><Settings2 /></button>
            {projectId ? <Link className="chat-project-context" href={`/projects/${encodeURIComponent(projectId)}`}><FolderKanban /><span>Project workspace</span></Link> : null}
          </header>
          <div className="chat-main"><ShennongThread projectId={projectId} initialPrompt={handoffPrompt} /></div>
        </div>
      </AppShell>
    </ShennongRuntimeProvider>
  );
}
