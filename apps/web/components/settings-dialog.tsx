"use client";

import { FormEvent, useCallback, useEffect, useState } from "react";
import {
  Archive,
  ArchiveRestore,
  Bot,
  Brain,
  Check,
  CircleOff,
  MonitorCog,
  Pencil,
  Plus,
  Puzzle,
  Trash2,
  WandSparkles,
} from "lucide-react";
import {
  createAiProvider,
  createAgentSkill,
  createGuidedAgentSkillDraft,
  deleteAiProvider,
  listAgentSkills,
  listAiProviders,
  updateAgentSkill,
  updateAiProvider,
  type AiProviderRecord,
  type AgentSkillRecord,
} from "@/lib/api/adapter";
import { type SettingsSection } from "@/lib/settings-route";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { MemoryManager } from "@/components/memory-manager";

export type { SettingsSection } from "@/lib/settings-route";
type Session = { authenticated: boolean; user_id: string; role: string } | null;

const sections = [
  ["general", "General", MonitorCog],
  ["models", "Models", Bot],
  ["skills", "Skills", Puzzle],
  ["memory", "Memory", Brain],
] as const;

const providerDefaults: Record<AiProviderRecord["providerType"], { baseUrl: string }> = {
  openai: { baseUrl: "https://api.openai.com/v1" },
  deepseek: { baseUrl: "https://api.deepseek.com" },
  ollama: { baseUrl: "http://host.docker.internal:11434/v1" },
  "llama-cpp": { baseUrl: "http://host.docker.internal:8081/v1" },
  "openai-compatible": { baseUrl: "" },
};

const providerLabels: Record<AiProviderRecord["providerType"], string> = {
  openai: "OpenAI",
  deepseek: "DeepSeek",
  ollama: "Ollama",
  "llama-cpp": "llama.cpp",
  "openai-compatible": "OpenAI-compatible",
};

export function SettingsDialog({ open, onOpenChange, onSectionChange, session, initialSection = "general" }: { open: boolean; onOpenChange: (open: boolean) => void; onSectionChange?: (section: SettingsSection) => void; session: Session; initialSection?: SettingsSection }) {
  const [section, setSection] = useState<SettingsSection>("general");
  useEffect(() => { if (open) setSection(initialSection); }, [initialSection, open]);
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="settings-dialog" showCloseButton>
        <DialogTitle className="sr-only">Settings</DialogTitle>
        <DialogDescription className="sr-only">Configure Shennong workspace and account preferences.</DialogDescription>
        <aside className="settings-nav" aria-label="Settings sections">
          <h2>Settings</h2>
          {sections.map(([value, label, Icon]) => (
            <button key={value} className={section === value ? "active" : ""} onClick={() => { setSection(value); onSectionChange?.(value); }}>
              <Icon />
              <span>{label}</span>
            </button>
          ))}
        </aside>
        <div className="settings-content">
          {section === "general" && <GeneralSettings />}
          {section === "models" && <ModelSettings authenticated={Boolean(session?.authenticated)} />}
          {section === "skills" && <SkillsSettings authenticated={Boolean(session?.authenticated)} />}
          {section === "memory" && <MemorySettings authenticated={Boolean(session?.authenticated)} />}
        </div>
      </DialogContent>
    </Dialog>
  );
}

function GeneralSettings() {
  const [density, setDensity] = useState<"comfortable" | "compact">("comfortable");
  useEffect(() => {
    const saved = window.localStorage.getItem("shennong.interface-density");
    setDensity(saved === "compact" ? "compact" : "comfortable");
  }, []);
  function update(value: "comfortable" | "compact") {
    setDensity(value);
    window.localStorage.setItem("shennong.interface-density", value);
    document.documentElement.dataset.density = value;
  }
  return (
    <SettingsPanel title="General">
      <SettingRow label="Interface density">
        <div className="settings-segmented" role="group" aria-label="Interface density">
          <button className={density === "comfortable" ? "active" : ""} onClick={() => update("comfortable")}>Comfortable</button>
          <button className={density === "compact" ? "active" : ""} onClick={() => update("compact")}>Compact</button>
        </div>
      </SettingRow>
      <SettingRow label="Search shortcut"><kbd>Ctrl / ⌘ K</kbd></SettingRow>
    </SettingsPanel>
  );
}

function ModelSettings({ authenticated }: { authenticated: boolean }) {
  const [providers, setProviders] = useState<AiProviderRecord[]>([]);
  const [editing, setEditing] = useState<AiProviderRecord | null>(null);
  const [showForm, setShowForm] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const load = useCallback(async () => {
    if (!authenticated) return;
    setLoading(true);
    setError("");
    try { setProviders(await listAiProviders()); }
    catch (reason) { setError(reason instanceof Error ? reason.message : "Model connections could not be loaded"); }
    finally { setLoading(false); }
  }, [authenticated]);
  useEffect(() => { void load(); }, [load]);
  async function remove(provider: AiProviderRecord) {
    if (!window.confirm(`Remove ${provider.name}?`)) return;
    setError("");
    try { await deleteAiProvider(provider.id); await load(); window.dispatchEvent(new Event("shennong:providers-updated")); }
    catch (reason) { setError(reason instanceof Error ? reason.message : "Model connection could not be removed"); }
  }
  return (
    <SettingsPanel title="Models" action={authenticated ? <button className="settings-command" onClick={() => { setEditing(null); setShowForm(true); }}><Plus />Add model</button> : undefined}>
      {!authenticated ? <SettingsEmpty title="Sign in to configure model connections." /> : loading ? <SettingsEmpty title="Loading model connections…" /> : (
        <>
          {error && <div className="settings-error" role="alert">{error}</div>}
          <div className="model-list">
            {providers.map((provider) => (
              <div className="model-row" key={provider.id}>
                <span className="model-logo"><Bot /></span>
                <span className="model-copy"><strong>{provider.name}</strong><small>{provider.providerType} · {provider.model || "No model selected"} · {provider.dataPolicy === "allow_private" ? "Private data allowed" : "Public Resources only"}</small></span>
                {provider.isDefault ? <span className="settings-status"><Check />Default</span> : null}
                <button className="settings-icon" aria-label={`Edit ${provider.name}`} onClick={() => { setEditing(provider); setShowForm(true); }}><Pencil /></button>
                <button className="settings-icon danger" aria-label={`Remove ${provider.name}`} onClick={() => void remove(provider)}><Trash2 /></button>
              </div>
            ))}
            {providers.length === 0 && !error ? <SettingsEmpty title="No model connections yet." /> : null}
          </div>
          {showForm ? <ProviderForm provider={editing} onCancel={() => setShowForm(false)} onSaved={async () => { setShowForm(false); await load(); window.dispatchEvent(new Event("shennong:providers-updated")); }} /> : null}
        </>
      )}
    </SettingsPanel>
  );
}

function ProviderForm({ provider, onCancel, onSaved }: { provider: AiProviderRecord | null; onCancel: () => void; onSaved: () => Promise<void> }) {
  const initialType = provider?.providerType ?? "openai";
  const [type, setType] = useState<AiProviderRecord["providerType"]>(initialType);
  const [baseUrl, setBaseUrl] = useState(provider?.baseUrl ?? providerDefaults[initialType].baseUrl);
  const [apiKey, setApiKey] = useState("");
  const [model, setModel] = useState(provider?.model ?? "");
  const [dataPolicy, setDataPolicy] = useState<AiProviderRecord["dataPolicy"]>(provider?.dataPolicy ?? "public_only");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!model.trim()) { setError("Enter the model ID exposed by this provider."); return; }
    setBusy(true);
    setError("");
    const form = new FormData(event.currentTarget);
    const value = {
      name: `${providerLabels[type]} · ${model}`,
      provider_kind: type,
      base_url: baseUrl.trim(),
      model: model.trim(),
      data_policy: dataPolicy,
      enabled: true,
      is_default: form.get("is_default") === "on",
      ...(apiKey.trim() ? { api_key: apiKey.trim() } : {}),
    };
    try {
      if (provider) {
        const { provider_kind: _providerKind, ...update } = value;
        void _providerKind;
        await updateAiProvider(provider.id, update);
      }
      else await createAiProvider(value);
      await onSaved();
    } catch (reason) { setError(reason instanceof Error ? reason.message : "Model connection could not be saved"); }
    finally { setBusy(false); }
  }
  function changeType(value: AiProviderRecord["providerType"]) {
    setType(value);
    setBaseUrl(providerDefaults[value].baseUrl);
    setApiKey("");
    setModel("");
    setError("");
  }
  return (
    <form className="provider-form" onSubmit={submit}>
      <div className="provider-form-heading"><h3>{provider ? "Edit model connection" : "Add model connection"}</h3><button type="button" className="settings-text-button" onClick={onCancel}>Cancel</button></div>
      <div className="provider-form-grid">
        <label className="wide">Provider<select value={type} onChange={(event) => changeType(event.target.value as AiProviderRecord["providerType"])} autoFocus disabled={Boolean(provider)}><option value="openai">OpenAI</option><option value="deepseek">DeepSeek</option><option value="ollama">Ollama</option><option value="llama-cpp">llama.cpp</option><option value="openai-compatible">OpenAI-compatible</option></select></label>
        {type !== "ollama" ? <label className="wide">API key<input value={apiKey} onChange={(event) => setApiKey(event.target.value)} type="password" autoComplete="off" placeholder={provider?.hasApiKey ? "Leave blank to use the saved key" : "Paste API key"} /></label> : null}
        {type === "openai-compatible" ? <label className="wide">Base URL<input value={baseUrl} onChange={(event) => setBaseUrl(event.target.value)} placeholder="https://provider.example/v1" required /></label> : null}
        <label className="wide">Model ID<input value={model} onChange={(event) => setModel(event.target.value)} placeholder="e.g. gpt-5-mini" required /></label>
        <label className="wide">Data access<select value={dataPolicy} onChange={(event) => setDataPolicy(event.target.value as AiProviderRecord["dataPolicy"])}><option value="public_only">Public Resources only</option><option value="allow_private">Allow private data</option></select></label>
        <label className="provider-checkbox"><input name="is_default" type="checkbox" defaultChecked={provider?.isDefault ?? false} />Use by default</label>
      </div>
      {dataPolicy === "allow_private" ? <div className="provider-policy-warning" role="note">Tool results and attachment metadata may be sent to this model provider.</div> : null}
      {error ? <div className="settings-error" role="alert">{error}</div> : null}
      <div className="provider-form-actions"><button type="button" className="settings-secondary" onClick={onCancel}>Cancel</button><button className="settings-primary" disabled={busy || !model.trim()}>{busy ? "Saving…" : "Save model"}</button></div>
    </form>
  );
}

function SkillsSettings({ authenticated }: { authenticated: boolean }) {
  const [skills, setSkills] = useState<AgentSkillRecord[]>([]);
  const [mode, setMode] = useState<"list" | "custom" | "guided">("list");
  const [editing, setEditing] = useState<AgentSkillRecord | null>(null);
  const [loading, setLoading] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const load = useCallback(async () => {
    if (!authenticated) return;
    setLoading(true);
    setError("");
    try { setSkills(await listAgentSkills()); }
    catch (reason) { setError(reason instanceof Error ? reason.message : "Skills could not be loaded"); }
    finally { setLoading(false); }
  }, [authenticated]);
  useEffect(() => { void load(); }, [load]);

  async function saveCustom(event: FormEvent<HTMLFormElement>, skill?: AgentSkillRecord) {
    event.preventDefault();
    setBusy(true);
    setError("");
    const form = new FormData(event.currentTarget);
    const name = String(form.get("name") ?? "").trim();
    const description = String(form.get("description") ?? "").trim();
    const content = String(form.get("content") ?? "").trim();
    try {
      if (skill) await updateAgentSkill(skill.id, { name, description, content, status: skill.status, change_note: "Updated in WebUI" });
      else await createAgentSkill({ name, description, content });
      setMode("list");
      setEditing(null);
      await load();
      window.dispatchEvent(new Event("shennong:skills-updated"));
    } catch (reason) { setError(reason instanceof Error ? reason.message : "Skill could not be saved"); }
    finally { setBusy(false); }
  }

  async function createGuidedDraft(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setBusy(true);
    setError("");
    const form = new FormData(event.currentTarget);
    const lines = (name: string) => String(form.get(name) ?? "").split("\n").map((line) => line.trim()).filter(Boolean);
    try {
      const skill = await createGuidedAgentSkillDraft({
        name: String(form.get("name") ?? "").trim() || undefined,
        goal: String(form.get("goal") ?? "").trim(),
        constraints: lines("constraints"),
        workflow: lines("workflow"),
      });
      setEditing(skill);
      setMode("list");
      await load();
      window.dispatchEvent(new Event("shennong:skills-updated"));
    } catch (reason) { setError(reason instanceof Error ? reason.message : "Guided Skill draft could not be created"); }
    finally { setBusy(false); }
  }

  async function changeStatus(skill: AgentSkillRecord) {
    setError("");
    try {
      await updateAgentSkill(skill.id, { name: skill.name, description: skill.description, status: skill.status === "active" ? "disabled" : "active" });
      await load();
      window.dispatchEvent(new Event("shennong:skills-updated"));
    } catch (reason) { setError(reason instanceof Error ? reason.message : "Skill status could not be updated"); }
  }

  async function archive(skill: AgentSkillRecord) {
    setError("");
    try {
      await updateAgentSkill(skill.id, { name: skill.name, description: skill.description, status: skill.status === "archived" ? "draft" : "archived" });
      await load();
      window.dispatchEvent(new Event("shennong:skills-updated"));
    } catch (reason) { setError(reason instanceof Error ? reason.message : "Skill lifecycle could not be updated"); }
  }

  const actions = authenticated ? <div className="skill-header-actions"><button className="settings-secondary" onClick={() => { setEditing(null); setMode("guided"); }}><WandSparkles />Guided draft</button><button className="settings-command" onClick={() => { setEditing(null); setMode("custom"); }}><Plus />Add custom</button></div> : undefined;
  return (
    <SettingsPanel title="Skills" action={actions}>
      {!authenticated ? <SettingsEmpty title="Sign in to manage Agent Skills." /> : loading ? <SettingsEmpty title="Loading Skills…" /> : (
        <>
          {error ? <div className="settings-error" role="alert">{error}</div> : null}
          <div className="provider-policy-note"><strong>Versioned Skills</strong><p>Custom and guided Skills begin as drafts. Saving instructions creates an immutable revision; archive replaces destructive deletion.</p></div>
          {mode === "custom" ? <SkillEditor busy={busy} onCancel={() => setMode("list")} onSubmit={(event) => void saveCustom(event)} /> : null}
          {mode === "guided" ? <SkillGenerator busy={busy} onCancel={() => setMode("list")} onSubmit={(event) => void createGuidedDraft(event)} /> : null}
          {editing ? <SkillEditor skill={editing} busy={busy} onCancel={() => setEditing(null)} onSubmit={(event) => void saveCustom(event, editing)} /> : null}
          {mode === "list" && !editing ? <div className="skill-list">{skills.map((skill) => <div className="skill-row" key={skill.id}><div className="skill-row-main"><span className="model-logo"><Puzzle /></span><span className="model-copy"><strong>{skill.name}</strong><small>{skill.description || "No description"}</small><small>{skill.sourceKind.replace("_", " ")} · revision {skill.revision}</small></span><span className={`skill-status ${skill.status}`}>{skill.status}</span>{!skill.isBuiltin ? <>{skill.status !== "archived" ? <><button className="settings-icon" aria-label={`Edit ${skill.name}`} title="Edit Skill" onClick={() => setEditing(skill)}><Pencil /></button><button className="settings-icon" aria-label={`${skill.status === "active" ? "Disable" : "Activate"} ${skill.name}`} title={skill.status === "active" ? "Disable Skill" : "Activate Skill"} onClick={() => void changeStatus(skill)}>{skill.status === "active" ? <CircleOff /> : <Check />}</button></> : null}<button className="settings-icon" aria-label={`${skill.status === "archived" ? "Restore" : "Archive"} ${skill.name}`} title={skill.status === "archived" ? "Restore as draft" : "Archive Skill"} onClick={() => void archive(skill)}>{skill.status === "archived" ? <ArchiveRestore /> : <Archive />}</button></> : null}</div><details className="skill-instructions"><summary>View instructions</summary><pre>{skill.content}</pre></details></div>)}{skills.length === 0 && !error ? <SettingsEmpty title="No persisted Skills." /> : null}</div> : null}
        </>
      )}
    </SettingsPanel>
  );
}

function SkillEditor({ skill, busy, onCancel, onSubmit }: { skill?: AgentSkillRecord; busy: boolean; onCancel: () => void; onSubmit: (event: FormEvent<HTMLFormElement>) => void }) {
  return <form className="skill-editor" onSubmit={onSubmit}><div className="provider-form-heading"><h3>{skill ? "Edit Skill" : "Add custom Skill"}</h3><button type="button" className="settings-text-button" onClick={onCancel}>Cancel</button></div><label>Name<input name="name" defaultValue={skill?.name ?? ""} required autoFocus /></label><label>Description<input name="description" defaultValue={skill?.description ?? ""} /></label><label>Instructions (Markdown)<textarea name="content" defaultValue={skill?.content ?? ""} rows={10} required /></label><div className="provider-form-actions"><button type="button" className="settings-secondary" onClick={onCancel}>Cancel</button><button className="settings-primary" disabled={busy}>{busy ? "Saving…" : skill ? "Save revision" : "Add Skill"}</button></div></form>;
}

function SkillGenerator({ busy, onCancel, onSubmit }: { busy: boolean; onCancel: () => void; onSubmit: (event: FormEvent<HTMLFormElement>) => void }) {
  return <form className="skill-editor" onSubmit={onSubmit}><div className="provider-form-heading"><h3>Create a guided Skill draft</h3><button type="button" className="settings-text-button" onClick={onCancel}>Cancel</button></div><p className="provider-policy-warning">This builds a transparent Markdown template locally. It does not claim AI generation.</p><label>Name (optional)<input name="name" autoFocus /></label><label>Goal<textarea name="goal" rows={3} required placeholder="What should this Skill help the Agent accomplish?" /></label><label>Constraints, one per line<textarea name="constraints" rows={3} /></label><label>Workflow, one step per line<textarea name="workflow" rows={4} /></label><div className="provider-form-actions"><button type="button" className="settings-secondary" onClick={onCancel}>Cancel</button><button className="settings-primary" disabled={busy}><WandSparkles />{busy ? "Creating…" : "Create draft"}</button></div></form>;
}

function MemorySettings({ authenticated }: { authenticated: boolean }) {
  return <SettingsPanel title="Memory">{authenticated ? <MemoryManager /> : <SettingsEmpty title="Sign in to manage global memory." />}</SettingsPanel>;
}

function SettingsPanel({ title, action, children }: { title: string; action?: React.ReactNode; children: React.ReactNode }) {
  return <section className="settings-panel"><header><h2>{title}</h2>{action}</header><div className="settings-panel-body">{children}</div></section>;
}

function SettingRow({ label, description, children }: { label: string; description?: string; children: React.ReactNode }) {
  return <div className="settings-setting-row"><span><strong>{label}</strong>{description ? <small>{description}</small> : null}</span>{children}</div>;
}

function SettingsEmpty({ title }: { title: string }) { return <div className="settings-empty">{title}</div>; }
