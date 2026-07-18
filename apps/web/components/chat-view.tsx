"use client";

import Link from "next/link";
import { FolderKanban, Settings2 } from "lucide-react";
import { AppShell } from "@/components/app-shell";
import { ShennongRuntimeProvider } from "@/components/assistant-ui/runtime-provider";
import { ShennongThread, ThreadSkillSelector } from "@/components/assistant-ui/thread";

export function ChatView({ threadId, projectId }: { threadId?: string; projectId?: string }) {
  if (!projectId) {
    return (
      <AppShell active="chat">
        <div className="chat-workspace empty-conversation">
          <header className="chat-header">
            <span className="chat-model-button"><span>Shennong Agent</span></span>
          </header>
          <div className="chat-main">
            <section className="chat-empty">
              <span className="chat-empty-mark"><FolderKanban /></span>
              <h1>Choose a research project</h1>
              <p>Every conversation, analysis run, and result must stay inside an explicit Project permission boundary.</p>
              <div className="chat-auth-actions"><Link className="primary-button" href="/projects">Open projects</Link></div>
            </section>
          </div>
        </div>
      </AppShell>
    );
  }
  return (
    <ShennongRuntimeProvider initialThreadId={threadId} projectId={projectId}>
      <AppShell active="chat" assistantThreads>
        <div className="chat-workspace has-assistant-ui">
          <header className="chat-header">
            <button className="chat-model-button" onClick={() => window.dispatchEvent(new CustomEvent("shennong:open-settings", { detail: "models" }))}>
              <span>Shennong Agent</span><Settings2 />
            </button>
            <ThreadSkillSelector />
            {projectId ? <Link className="chat-project-context" href={`/projects/${encodeURIComponent(projectId)}`}><FolderKanban /><span>Project workspace</span></Link> : null}
          </header>
          <div className="chat-main"><ShennongThread /></div>
        </div>
      </AppShell>
    </ShennongRuntimeProvider>
  );
}
