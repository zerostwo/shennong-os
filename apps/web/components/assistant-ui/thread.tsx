"use client";

import {
  AttachmentPrimitive,
  ComposerPrimitive,
  MessagePrimitive,
  ThreadPrimitive,
  unstable_useMentionAdapter,
  unstable_useSlashCommandAdapter,
  useComposerRuntime,
  type MessageState,
} from "@assistant-ui/react";
import {
  useAgUiInterrupts,
  useAgUiSubmitInterruptResponses,
} from "@assistant-ui/react-ag-ui";
import { ArrowDown, ArrowLeft, ArrowUp, AtSign, Brain, Check, ChevronRight, CircleStop, Database, FileText, FolderKanban, LoaderCircle, Paperclip, Puzzle, ShieldAlert, Sparkles, TerminalSquare, X } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { ChatMarkdown } from "@/components/chat-markdown";
import { useShennongAssistantRuntime } from "@/components/assistant-ui/runtime-provider";
import { disableThreadSkill, enableThreadSkill, listThreadSkills, type AgentSkillRecord } from "@/lib/api/adapter";

function textFromPart(part: MessageState["content"][number]) {
  return part.type === "text" ? part.text : "";
}

function reasoningFromPart(part: MessageState["content"][number]) {
  return part.type === "reasoning" ? part.text : "";
}

type Clarification = { options: string[]; allowOther: boolean };

function extractClarification(text: string): { text: string; clarification?: Clarification } {
  const match = text.match(/<shennong-clarification>([\s\S]*?)<\/shennong-clarification>/i);
  if (!match) return { text };
  try {
    const value = JSON.parse(match[1]) as Partial<Clarification>;
    const options = Array.isArray(value.options)
      ? value.options.filter((item): item is string => typeof item === "string" && item.trim().length > 0).slice(0, 3)
      : [];
    if (options.length < 2) return { text };
    return {
      text: text.replace(match[0], "").trim(),
      clarification: { options, allowOther: value.allowOther !== false },
    };
  } catch {
    return { text };
  }
}

function ClarificationCard({ value }: { value: Clarification }) {
  const composer = useComposerRuntime();
  const [other, setOther] = useState("");
  const submit = (answer: string) => {
    const normalized = answer.trim();
    if (!normalized) return;
    composer.setText(normalized);
    queueMicrotask(() => composer.send());
  };
  return (
    <section className="aui-clarification" aria-label="Choose a response">
      <div className="aui-clarification-options">
        {value.options.map((option, index) => (
          <button key={option} type="button" onClick={() => submit(option)}>
            <span>{option}</span>{index === 0 ? <small>Recommended</small> : null}<ChevronRight />
          </button>
        ))}
      </div>
      {value.allowOther ? (
        <form onSubmit={(event) => { event.preventDefault(); submit(other); }}>
          <input value={other} onChange={(event) => setOther(event.target.value)} placeholder="Type another answer…" aria-label="Another answer" />
          <button type="submit" disabled={!other.trim()} aria-label="Send another answer"><ArrowUp /></button>
        </form>
      ) : null}
    </section>
  );
}

function ToolCard({ part }: { part: Extract<MessageState["content"][number], { type: "tool-call" }> }) {
  const running = !("result" in part) || part.result === undefined;
  const failed = part.isError === true;
  const Icon = part.toolName.startsWith("db.") ? Database : part.toolName.startsWith("artifact.") ? FileText : TerminalSquare;
  return (
    <section className={`aui-tool-card ${running ? "running" : failed ? "failed" : "complete"}`}>
      <header><Icon /><strong>{part.toolName}</strong><span>{running ? <><LoaderCircle />Running</> : failed ? <><X />Failed</> : <><Check />Complete</>}</span></header>
      {part.args && Object.keys(part.args).length ? <details><summary>Inputs</summary><pre>{JSON.stringify(part.args, null, 2)}</pre></details> : null}
      {"result" in part && part.result !== undefined ? <details><summary>Result</summary><pre>{typeof part.result === "string" ? part.result : JSON.stringify(part.result, null, 2)}</pre></details> : null}
    </section>
  );
}

function ShennongMessage({ message }: { message: MessageState }) {
  if (message.role === "system") return null;
  const rawText = message.content.map(textFromPart).join("");
  const reasoning = message.content.map(reasoningFromPart).join("");
  const { text, clarification } = extractClarification(rawText);
  const tools = message.content.filter((part): part is Extract<typeof part, { type: "tool-call" }> => part.type === "tool-call");
  return (
    <MessagePrimitive.Root className={`aui-message ${message.role}`}>
      <div className="aui-message-body">
        {reasoning ? <details className="chat-reasoning"><summary><Brain /><strong>Reasoning</strong><span>Model summary</span><ChevronRight /></summary><ChatMarkdown>{reasoning}</ChatMarkdown></details> : null}
        {text ? <ChatMarkdown>{text}</ChatMarkdown> : null}
        {tools.map((part) => <ToolCard key={part.toolCallId} part={part} />)}
        {clarification ? <ClarificationCard value={clarification} /> : null}
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

function ComposerAttachment() {
  return (
    <AttachmentPrimitive.Root className="aui-attachment-chip">
      <Paperclip />
      <AttachmentPrimitive.Name />
      <AttachmentPrimitive.Remove className="aui-attachment-remove" aria-label="Remove attachment"><X /></AttachmentPrimitive.Remove>
    </AttachmentPrimitive.Root>
  );
}

function TriggerItems() {
  return (
    <>
      <ComposerPrimitive.Unstable_TriggerPopoverCategories className="aui-trigger-items">
        {(categories) => categories.map((category) => (
          <ComposerPrimitive.Unstable_TriggerPopoverCategoryItem key={category.id} categoryId={category.id} className="aui-trigger-item">
            <FolderKanban /><span><strong>{category.label}</strong><small>Browse suggestions</small></span><ChevronRight />
          </ComposerPrimitive.Unstable_TriggerPopoverCategoryItem>
        ))}
      </ComposerPrimitive.Unstable_TriggerPopoverCategories>
      <ComposerPrimitive.Unstable_TriggerPopoverItems className="aui-trigger-items">
        {(items) => (
          <>
            <ComposerPrimitive.Unstable_TriggerPopoverBack className="aui-trigger-back"><ArrowLeft />Back</ComposerPrimitive.Unstable_TriggerPopoverBack>
            {items.map((item, index) => (
              <ComposerPrimitive.Unstable_TriggerPopoverItem key={item.id} item={item} index={index} className="aui-trigger-item">
                {item.type === "resource" ? <Database /> : item.type === "command" ? <TerminalSquare /> : <Sparkles />}
                <span><strong>{item.label}</strong>{item.description ? <small>{item.description}</small> : null}</span>
              </ComposerPrimitive.Unstable_TriggerPopoverItem>
            ))}
          </>
        )}
      </ComposerPrimitive.Unstable_TriggerPopoverItems>
    </>
  );
}

function ComposerTriggers() {
  const runtime = useShennongAssistantRuntime();
  const composer = useComposerRuntime();
  const mentionCategories = useMemo(() => [
    {
      id: "resources",
      label: "Resources",
      items: [
        { id: "toil", type: "resource", label: "TOIL pan-cancer", description: "TCGA, TARGET and GTEx RNA-seq Resource" },
        { id: "catalog", type: "resource", label: "Shennong DB catalog", description: "Discover governed public Resources" },
      ],
    },
    {
      id: "capabilities",
      label: "Capabilities",
      items: [
        { id: "runtime", type: "capability", label: "R Runtime", description: runtime?.projectId ? "Run bounded R jobs in this Project" : "Choose a Project before executing code" },
        { id: "skills", type: "capability", label: "Skills", description: "Use the Skills enabled for this task" },
      ],
    },
  ], [runtime?.projectId]);
  const mention = unstable_useMentionAdapter({ categories: mentionCategories, includeModelContextTools: false });
  const setPrompt = useCallback((prompt: string) => queueMicrotask(() => composer.setText(prompt)), [composer]);
  const slash = unstable_useSlashCommandAdapter({
    removeOnExecute: true,
    commands: [
      { id: "data", label: "/data", description: "Discover a Shennong DB Resource", execute: () => setPrompt("Discover the governed Shennong DB Resources relevant to ") },
      { id: "plot", label: "/plot", description: "Create an R/ggplot2 figure in Runtime", execute: () => setPrompt("Create a reproducible ggplot2 figure in the governed Runtime using ") },
      { id: "toil", label: "/toil", description: "Inspect the TOIL pan-cancer Resource", execute: () => setPrompt("Inspect the TOIL pan-cancer Resource and explain its cohorts, expression units, and supported queries.") },
      { id: "skills", label: "/skills", description: "Review task capabilities", execute: () => setPrompt("List the Skills and governed tools available in this conversation, including any Project-only requirements.") },
    ],
  });
  return (
    <>
      <ComposerPrimitive.Unstable_TriggerPopover char="@" adapter={mention.adapter} className="aui-trigger-popover" aria-label="Mention suggestions">
        <ComposerPrimitive.Unstable_TriggerPopover.Directive {...mention.directive} />
        <TriggerItems />
      </ComposerPrimitive.Unstable_TriggerPopover>
      <ComposerPrimitive.Unstable_TriggerPopover char="/" adapter={slash.adapter} className="aui-trigger-popover" aria-label="Command suggestions">
        <ComposerPrimitive.Unstable_TriggerPopover.Action {...slash.action} />
        <TriggerItems />
      </ComposerPrimitive.Unstable_TriggerPopover>
    </>
  );
}

function EmptyState({ projectName }: { projectName?: string }) {
  const composer = useComposerRuntime();
  const choose = (text: string) => {
    composer.setText(text);
    queueMicrotask(() => composer.send());
  };
  return (
    <section className="chat-empty-state">
      <h1>What are we working on?</h1>
      <p>{projectName ? `Ask about ${projectName}, its data, analysis plan, or results.` : "Discover Shennong DB data here. Choose a Project when you want to run R or create artifacts."}</p>
      <div className="chat-empty-prompts">
        <button type="button" onClick={() => choose("Inspect the TOIL pan-cancer Resource and summarize its cohorts, expression units, dimensions, and supported queries.")}><Database />Inspect TOIL pan-cancer data</button>
        <button type="button" onClick={() => choose("Create a reproducible ggplot2 figure using the R built-in mtcars dataset and save the artifact in this Project.")}><Sparkles />Create a ggplot2 figure</button>
      </div>
    </section>
  );
}

export function ShennongThread({ projectName }: { projectName?: string }) {
  return (
    <ThreadPrimitive.Root className="aui-thread-root">
      <ThreadPrimitive.Viewport className="aui-thread-viewport">
        <ThreadPrimitive.Empty>
          <EmptyState projectName={projectName} />
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
            <ComposerPrimitive.Attachments components={{ Attachment: ComposerAttachment }} />
            <ComposerPrimitive.Unstable_TriggerPopoverRoot>
              <ComposerPrimitive.Input className="aui-composer-input" placeholder={projectName ? `Ask about ${projectName} · @ mention · / command` : "Ask Shennong · @ mention · / command"} rows={1} autoFocus />
              <ComposerTriggers />
            </ComposerPrimitive.Unstable_TriggerPopoverRoot>
            <div className="chat-composer-toolbar">
              <ComposerPrimitive.AddAttachment className="aui-composer-tool" aria-label="Attach a file" title="Attach a text file or image"><Paperclip /></ComposerPrimitive.AddAttachment>
              <span className="aui-composer-hint"><AtSign /> mention <span>/</span> commands</span>
              <span className="aui-composer-note">Code runs in the isolated Runtime with approval.</span>
              <ComposerPrimitive.Send className="aui-send" aria-label="Send message"><ArrowUp /></ComposerPrimitive.Send>
              <ComposerPrimitive.Cancel className="aui-cancel" aria-label="Stop generation"><CircleStop /></ComposerPrimitive.Cancel>
            </div>
          </ComposerPrimitive.Root>
        </ThreadPrimitive.ViewportFooter>
      </ThreadPrimitive.Viewport>
    </ThreadPrimitive.Root>
  );
}
