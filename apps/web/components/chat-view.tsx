"use client";

import Link from "next/link";
import { Brain, ChevronDown, FolderKanban, Settings2 } from "lucide-react";
import { AppShell } from "@/components/app-shell";
import { ShennongRuntimeProvider, useShennongAssistantRuntime, type ThinkingLevel } from "@/components/assistant-ui/runtime-provider";
import { ShennongThread, ThreadSkillSelector } from "@/components/assistant-ui/thread";

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
          <div className="chat-main"><ShennongThread projectId={projectId} initialPrompt={initialPrompt} /></div>
        </div>
      </AppShell>
    </ShennongRuntimeProvider>
  );
}
