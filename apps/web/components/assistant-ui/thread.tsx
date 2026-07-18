"use client";

import {
  ComposerPrimitive,
  MessagePrimitive,
  ThreadPrimitive,
  type MessageState,
  useAssistantRuntime,
} from "@assistant-ui/react";
import {
  useAgUiInterrupts,
  useAgUiSubmitInterruptResponses,
} from "@assistant-ui/react-ag-ui";
import { useQueryClient } from "@tanstack/react-query";
import { ArrowDown, ArrowUp, Bot, Check, CircleStop, Database, FileText, LoaderCircle, Paperclip, Puzzle, ShieldAlert, X } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { ChatMarkdown } from "@/components/chat-markdown";
import { useShennongAssistantRuntime } from "@/components/assistant-ui/runtime-provider";
import { disableThreadSkill, enableThreadSkill, listThreadSkills, type AgentSkillRecord } from "@/lib/api/adapter";
import { buildProjectUploadPrompt, registerProjectFiles } from "@/lib/project-upload";
import projectStyles from "@/components/project-ui.module.css";
import { StructuredValue } from "@/components/structured-value";

function textFromPart(part: MessageState["content"][number]) {
  return part.type === "text" ? part.text : "";
}

function ToolCard({ part }: { part: Extract<MessageState["content"][number], { type: "tool-call" }> }) {
  const running = !("result" in part) || part.result === undefined;
  const failed = part.isError === true;
  const Icon = part.toolName.startsWith("dataset.") ? Database : part.toolName.startsWith("artifact.") ? FileText : Bot;
  return (
    <section className={`aui-tool-card ${running ? "running" : failed ? "failed" : "complete"}`}>
      <header><Icon /><strong>{part.toolName}</strong><span>{running ? <><LoaderCircle />Running</> : failed ? <><X />Failed</> : <><Check />Complete</>}</span></header>
      {part.args && Object.keys(part.args).length ? <details><summary>Inputs</summary><div className="tool-structured-value"><StructuredValue value={part.args} /></div></details> : null}
      {"result" in part && part.result !== undefined ? <details><summary>Result</summary><div className="tool-structured-value"><StructuredValue value={part.result} /></div></details> : null}
    </section>
  );
}

function ShennongMessage({ message }: { message: MessageState }) {
  if (message.role === "system") return null;
  const text = message.content.map(textFromPart).join("");
  const tools = message.content.filter((part): part is Extract<typeof part, { type: "tool-call" }> => part.type === "tool-call");
  return (
    <MessagePrimitive.Root className={`aui-message ${message.role}`}>
      {message.role === "assistant" ? <span className="assistant-avatar"><Bot /></span> : null}
      <div className="aui-message-body">
        {text ? <ChatMarkdown>{text}</ChatMarkdown> : null}
        {tools.map((part) => <ToolCard key={part.toolCallId} part={part} />)}
      </div>
    </MessagePrimitive.Root>
  );
}

export function InterruptPanel() {
  const interrupts = useAgUiInterrupts();
  const submit = useAgUiSubmitInterruptResponses();
  const [busy, setBusy] = useState(false);
  if (!interrupts.length) return null;
  const resolve = async (approved: boolean) => {
    setBusy(true);
    try {
      await submit(interrupts.map((interrupt) => ({
        interruptId: interrupt.id,
        status: approved ? "resolved" as const : "cancelled" as const,
        ...(approved ? { payload: { approved: true } } : {}),
      })));
    } finally { setBusy(false); }
  };
  return (
    <section className="aui-interrupt" aria-live="polite">
      <ShieldAlert />
      <div><strong>Approval required</strong><p>{interrupts.map((item) => item.message ?? item.reason).join(" · ")}</p></div>
      <button disabled={busy} onClick={() => void resolve(false)}>Reject</button>
      <button className="primary-button" disabled={busy} onClick={() => void resolve(true)}>Approve</button>
    </section>
  );
}

export function ThreadSkillSelector() {
  const runtime = useShennongAssistantRuntime();
  const [skills, setSkills] = useState<AgentSkillRecord[]>([]);
  const [busyId, setBusyId] = useState("");
  const [error, setError] = useState("");
  const load = useCallback(async () => {
    if (!runtime?.hasPersistedThread) return;
    try {
      setError("");
      setSkills(await listThreadSkills(runtime.activeThreadId));
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "Skills could not be loaded");
    }
  }, [runtime?.activeThreadId, runtime?.hasPersistedThread]);
  useEffect(() => { void load(); }, [load]);
  useEffect(() => {
    const refresh = () => void load();
    window.addEventListener("shennong:skills-updated", refresh);
    return () => window.removeEventListener("shennong:skills-updated", refresh);
  }, [load]);

  if (!runtime?.hasPersistedThread) {
    return <button className="thread-skill-button" disabled title="Send the first message before selecting Skills"><Puzzle /><span>Skills</span></button>;
  }
  const selectable = skills.filter((skill) => skill.status === "active");
  const selected = selectable.filter((skill) => skill.enabled).length;
  async function toggle(skill: AgentSkillRecord) {
    if (!runtime) return;
    setBusyId(skill.id);
    setError("");
    try {
      if (skill.enabled) await disableThreadSkill(runtime.activeThreadId, skill.id);
      else await enableThreadSkill(runtime.activeThreadId, skill.id);
      await load();
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "Skill selection could not be updated");
    } finally { setBusyId(""); }
  }
  return (
    <details className="thread-skill-picker">
      <summary className="thread-skill-button"><Puzzle /><span>Skills</span>{selected ? <b>{selected}</b> : null}</summary>
      <section className="thread-skill-menu">
        <header><strong>Skills for this task</strong><small>Active versions are pinned when selected.</small></header>
        {error ? <p className="thread-skill-error" role="alert">{error}</p> : null}
        {selectable.map((skill) => <label key={skill.id} className="thread-skill-option"><input type="checkbox" checked={skill.enabled} disabled={busyId === skill.id} onChange={() => void toggle(skill)} /><span><strong>{skill.name}</strong><small>{skill.sourceKind.replace("_", " ")} · v{skill.enabled ? skill.selectedVersion ?? skill.revision : skill.revision}</small></span>{busyId === skill.id ? <LoaderCircle className="spin" /> : null}</label>)}
        {!selectable.length && !error ? <p className="thread-skill-empty">No active Skills. Activate a draft in Settings.</p> : null}
      </section>
    </details>
  );
}

function ComposerPrefill({ prompt }: { prompt?: string }) {
  const runtime = useAssistantRuntime();
  const applied = useRef("");
  useEffect(() => {
    if (!prompt || applied.current === prompt) return;
    if (!runtime.thread.composer.getState().text.trim()) runtime.thread.composer.setText(prompt);
    applied.current = prompt;
  }, [prompt, runtime]);
  return null;
}

function ProjectFileUploadButton({ projectId }: { projectId: string }) {
  const inputRef = useRef<HTMLInputElement>(null);
  const runtime = useAssistantRuntime();
  const queryClient = useQueryClient();
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");
  const [error, setError] = useState("");

  async function select(files: File[]) {
    if (!files.length) return;
    if (files.length > 20) {
      setError("Upload no more than 20 related files at once.");
      return;
    }
    const invalid = files.find((file) => file.size === 0 || file.size > 50 * 1024 * 1024 * 1024);
    if (invalid) {
      setError(`${invalid.name} must be non-empty and no larger than 50 GiB.`);
      return;
    }
    setBusy(true);
    setError("");
    setMessage("");
    try {
      const background = runtime.thread.composer.getState().text.slice(0, 4000);
      const result = await registerProjectFiles(projectId, files, background);
      runtime.thread.composer.setText(buildProjectUploadPrompt(result, background));
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: ["projects", projectId, "context-pack"] }),
        queryClient.invalidateQueries({ queryKey: ["projects", projectId, "resources"] }),
      ]);
      setMessage(`${result.resourceName} is registered. Review the message and send it to the Agent.`);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "Files could not be uploaded");
    } finally {
      setBusy(false);
      if (inputRef.current) inputRef.current.value = "";
    }
  }

  return <>
    <input ref={inputRef} type="file" multiple hidden onChange={(event) => void select(Array.from(event.target.files ?? []))} />
    <button type="button" className={projectStyles.composerAttach} disabled={busy} onClick={() => inputRef.current?.click()} aria-label="Attach Project files" title="Upload files into this Project">
      {busy ? <LoaderCircle className="spin" /> : <Paperclip />}
    </button>
    {message ? <span className={projectStyles.composerUploadStatus} role="status">{message}<button type="button" onClick={() => setMessage("")} aria-label="Dismiss upload status"><X /></button></span> : null}
    {error ? <span className={projectStyles.composerUploadError} role="alert">{error}<button type="button" onClick={() => setError("")} aria-label="Dismiss upload error"><X /></button></span> : null}
  </>;
}

export function ShennongThread({ projectName, projectId, initialPrompt }: { projectName?: string; projectId?: string; initialPrompt?: string }) {
  return (
    <ThreadPrimitive.Root className="aui-thread-root">
      <ComposerPrefill prompt={initialPrompt} />
      <ThreadPrimitive.Viewport className="aui-thread-viewport">
        <ThreadPrimitive.Empty>
          <section className="chat-empty-state">
            <span className="chat-empty-icon"><Bot /></span>
            <h1>What can I help you analyze?</h1>
            <p>{projectName ? `Ask about ${projectName}, its data, analysis plan, or results.` : "Ask a biomedical question, discover governed data, or start a reproducible analysis."}</p>
          </section>
        </ThreadPrimitive.Empty>
        <div className="aui-message-list">
          <ThreadPrimitive.Messages>
            {({ message }) => <ShennongMessage message={message} />}
          </ThreadPrimitive.Messages>
        </div>
        <ThreadPrimitive.ViewportFooter className="aui-viewport-footer">
          <ThreadPrimitive.ScrollToBottom className="aui-scroll-bottom" aria-label="Scroll to latest message"><ArrowDown /></ThreadPrimitive.ScrollToBottom>
          <InterruptPanel />
          <ComposerPrimitive.Root className="chat-composer aui-composer">
            <ComposerPrimitive.Input className="aui-composer-input" placeholder={projectName ? `Ask about ${projectName}` : "Ask Shennong"} rows={1} autoFocus />
            <div className="chat-composer-toolbar">
              <span className="aui-composer-note">Generated code runs in an isolated Runtime.</span>
              {projectId ? <ProjectFileUploadButton projectId={projectId} /> : null}
              <ComposerPrimitive.Send className="aui-send" aria-label="Send message"><ArrowUp /></ComposerPrimitive.Send>
              <ComposerPrimitive.Cancel className="aui-cancel" aria-label="Stop generation"><CircleStop /></ComposerPrimitive.Cancel>
            </div>
          </ComposerPrimitive.Root>
        </ThreadPrimitive.ViewportFooter>
      </ThreadPrimitive.Viewport>
    </ThreadPrimitive.Root>
  );
}
