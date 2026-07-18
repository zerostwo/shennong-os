"use client";

import Link from "next/link";
import { FolderKanban, Settings2 } from "lucide-react";
import { AppShell } from "@/components/app-shell";
import { ShennongRuntimeProvider } from "@/components/assistant-ui/runtime-provider";
import { ShennongThread, ThreadSkillSelector } from "@/components/assistant-ui/thread";

export function ChatView({ threadId, projectId }: { threadId?: string; projectId?: string }) {
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
