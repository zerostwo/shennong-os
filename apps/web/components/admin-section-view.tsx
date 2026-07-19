"use client";

import Link from "next/link";
import { useCallback, useEffect, useMemo, useState } from "react";
import { RefreshCw, Search } from "lucide-react";
import {
  getAdminOverview,
  getHealth,
  getRegistrationPolicy,
  getSystemDependencies,
  listAdminModelProviders,
  listAuditEvents,
  listProviders,
  listUsers,
  updateRegistrationPolicy,
  type JsonRecord,
} from "@/lib/api/adapter";
import { AppShell, TinyBadge, TopBar } from "./app-shell";
import { StructuredValue } from "./structured-value";

export type Section = "dashboard" | "users" | "models" | "providers" | "monitoring" | "audit" | "security" | "backups";

const sectionCopy: Record<Section, [string, string]> = {
  dashboard: ["Admin overview", "Live control-plane activity and service state."],
  users: ["Users", "Review persisted identities, access level, and account state."],
  models: ["Model providers", "Read-only oversight for user-configured agent providers."],
  providers: ["Resource providers", "Provider manifests reported by the governed data service."],
  monitoring: ["System health", "Current dependency state reported by Shennong services."],
  audit: ["Audit events", "Recent persisted governance and security events."],
  security: ["Registration security", "Control whether new accounts can be created."],
  backups: ["Backups", "Backup capability and recovery readiness."],
};

type AdminData = { rows: JsonRecord[]; overview?: JsonRecord; health?: JsonRecord; dependencies?: JsonRecord; policy?: JsonRecord };

export function AdminSectionView({ section }: { section: Section }) {
  const [data, setData] = useState<AdminData>({ rows: [] });
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [updatedAt, setUpdatedAt] = useState("");
  const load = useCallback(async () => {
    setLoading(true);
    setError("");
    try {
      if (section === "dashboard") {
        const [overview, health, rows] = await Promise.all([getAdminOverview(), getHealth(), listAuditEvents()]);
        setData({ overview, health, rows: rows as JsonRecord[] });
      } else if (section === "users") setData({ rows: await listUsers() as JsonRecord[] });
      else if (section === "models") setData({ rows: await listAdminModelProviders() });
      else if (section === "providers") setData({ rows: await listProviders() as JsonRecord[] });
      else if (section === "monitoring") {
        const [health, dependencies] = await Promise.all([getHealth(), getSystemDependencies()]);
        setData({ health, dependencies, rows: [] });
      } else if (section === "audit") setData({ rows: await listAuditEvents() as JsonRecord[] });
      else if (section === "security") setData({ policy: await getRegistrationPolicy(), rows: [] });
      else setData({ rows: [] });
      setUpdatedAt(new Date().toLocaleTimeString());
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "Admin data could not be loaded");
    } finally { setLoading(false); }
  }, [section]);
  useEffect(() => { void load(); }, [load]);
  const [title, description] = sectionCopy[section];
  return <AppShell variant="admin" active={section}>
    <TopBar title={title} description={description} search={false} action={<div className="admin-refresh"><span>{updatedAt ? `Updated ${updatedAt}` : "Waiting for live data"}</span><button className="outline-button" onClick={() => void load()} disabled={loading}><RefreshCw />Refresh</button></div>} />
    <div className="admin-page">
      {error ? <div className="error-banner" role="alert"><strong>Live request failed</strong><span>{error}</span></div> : null}
      {loading ? <div className="admin-loading" aria-live="polite"><span /><span /><span /></div> : <AdminContent section={section} data={data} reload={load} />}
    </div>
  </AppShell>;
}

function AdminContent({ section, data, reload }: { section: Section; data: AdminData; reload: () => Promise<void> }) {
  if (section === "dashboard") return <Dashboard data={data} />;
  if (section === "users") return <Users rows={data.rows} />;
  if (section === "models") return <Models rows={data.rows} />;
  if (section === "providers") return <ResourceProviders rows={data.rows} />;
  if (section === "monitoring") return <Monitoring health={data.health ?? {}} dependencies={data.dependencies ?? {}} />;
  if (section === "audit") return <Audit rows={data.rows} />;
  if (section === "security") return <Security policy={data.policy ?? {}} reload={reload} />;
  return <Unavailable title="Backup service is not configured" description="This deployment does not expose a backup API. Configure an external PostgreSQL and object-storage backup policy before enabling restore controls." />;
}

function Dashboard({ data }: { data: AdminData }) {
  const overview = data.overview ?? {};
  const backends = (data.health?.backends ?? data.health ?? {}) as JsonRecord;
  return <>
    <div className="admin-metric-strip">
      <Metric label="Users" value={overview.users} supporting={metricSupport(overview.active_users, "active")} />
      <Metric label="Projects" value={overview.projects} />
      <Metric label="Threads" value={overview.threads} />
      <Metric label="Runs" value={overview.runs} supporting={metricSupport(overview.active_runs, "active")} />
      <Metric label="Model providers" value={overview.model_providers} supporting={metricSupport(overview.enabled_model_providers, "enabled")} />
    </div>
    <div className="admin-layout-split">
      <section className="admin-panel"><PanelHeading title="Service state" description="Reported by the live health endpoint." /><StructuredValue value={backends} emptyLabel="No dependency state was reported." /></section>
      <section className="admin-panel"><PanelHeading title="Recent governance activity" description="Latest persisted audit records." /><AuditRows rows={data.rows.slice(0, 8)} compact /></section>
    </div>
  </>;
}

function Users({ rows }: { rows: JsonRecord[] }) {
  const [query, setQuery] = useState("");
  const filtered = useSearch(rows, query);
  return <section className="admin-panel"><PanelHeading title="Persisted identities" description={`${rows.length} accounts returned by the control plane.`} /><TableSearch value={query} onChange={setQuery} label="Search users" />
    <DataTable headings={["User", "Role", "Status", "Projects", "Sessions", "Updated"]} empty="No users match this search.">{filtered.map((row) => <tr key={String(row.id)}><td><Link className="admin-primary-link" href={`/admin/users/${encodeURIComponent(String(row.id))}`}>{display(row.display_name)}</Link><small>{display(row.email)}</small></td><td><TinyBadge tone={row.role === "admin" ? "purple" : "neutral"}>{display(row.role)}</TinyBadge></td><td><TinyBadge tone={row.status === "active" ? "green" : "amber"}>{display(row.status)}</TinyBadge></td><td>{display(row.owned_projects)}</td><td>{display(row.active_sessions)}</td><td>{formatDate(row.updated_at)}</td></tr>)}</DataTable>
  </section>;
}

function Models({ rows }: { rows: JsonRecord[] }) {
  const [query, setQuery] = useState(""); const filtered = useSearch(rows, query);
  return <section className="admin-panel"><PanelHeading title="Configured model endpoints" description="Secrets are never returned. Configuration remains owned by each user." /><TableSearch value={query} onChange={setQuery} label="Search model providers" />
    <DataTable headings={["Provider", "Owner", "Type", "Model", "Data policy", "State"]} empty="No model providers are configured.">{filtered.map((row) => <tr key={String(row.id)}><td><strong>{display(row.name)}</strong><small>{display(row.base_url)}</small></td><td>{display(row.owner_name)}<small>{display(row.owner_user_id)}</small></td><td>{display(row.provider_kind)}</td><td>{display(row.model)}</td><td>{display(row.data_policy)}</td><td><TinyBadge tone={row.enabled ? "green" : "amber"}>{row.enabled ? "Enabled" : "Disabled"}</TinyBadge>{row.is_default ? <small>Default for owner</small> : null}</td></tr>)}</DataTable>
  </section>;
}

function ResourceProviders({ rows }: { rows: JsonRecord[] }) {
  return <section className="admin-panel"><PanelHeading title="Available provider manifests" description="Read from the connected resource service." />
    <DataTable headings={["Provider", "Version", "Files", "Resource kind", "Title"]} empty="The resource service reported no provider manifests.">{rows.map((row, index) => { const schema = (row.resource_schema ?? {}) as JsonRecord; return <tr key={String(row.name ?? index)}><td><strong>{display(row.name)}</strong></td><td>{display(row.version)}</td><td>{Array.isArray(row.files) ? row.files.length : "Unavailable"}</td><td>{display(schema.kind)}</td><td>{display(schema.title)}</td></tr>; })}</DataTable>
  </section>;
}

function Monitoring({ health, dependencies }: { health: JsonRecord; dependencies: JsonRecord }) {
  return <div className="admin-layout-split"><section className="admin-panel"><PanelHeading title="Web and API health" description="Raw values are normalized into readable fields." /><StructuredValue value={health} emptyLabel="The health endpoint returned no details." /></section><section className="admin-panel"><PanelHeading title="Connected services" description="Dependency versions and availability from the Rust control plane." /><StructuredValue value={dependencies} emptyLabel="No dependency information is available." /></section></div>;
}

function Audit({ rows }: { rows: JsonRecord[] }) {
  const [query, setQuery] = useState("");
  const [page, setPage] = useState(0);
  const filtered = useSearch(rows, query);
  const pageSize = 25;
  const pageCount = Math.max(1, Math.ceil(filtered.length / pageSize));
  useEffect(() => setPage(0), [query]);
  const visible = filtered.slice(page * pageSize, (page + 1) * pageSize);
  const start = filtered.length ? page * pageSize + 1 : 0;
  const end = Math.min((page + 1) * pageSize, filtered.length);
  return <section className="admin-panel"><PanelHeading title="Governance trail" description="Up to 100 recent events returned by the audit API." /><TableSearch value={query} onChange={setQuery} label="Search audit events" /><AuditRows rows={visible} /><div className="admin-table-pagination"><span>{start}-{end} of {filtered.length}</span><div><button className="outline-button" disabled={page === 0} onClick={() => setPage((value) => Math.max(0, value - 1))}>Previous</button><span>Page {page + 1} of {pageCount}</span><button className="outline-button" disabled={page + 1 >= pageCount} onClick={() => setPage((value) => Math.min(pageCount - 1, value + 1))}>Next</button></div></div></section>;
}

function AuditRows({ rows, compact = false }: { rows: JsonRecord[]; compact?: boolean }) {
  return <DataTable headings={compact ? ["Time", "Action", "Target"] : ["Time", "Actor", "Action", "Target", "Request", "Details"]} empty="No audit events were returned.">{rows.map((row, index) => <tr key={String(row.id ?? index)}><td>{formatDate(row.created_at)}</td>{compact ? null : <td>{display(row.actor_user_id)}</td>}<td><strong>{display(row.action)}</strong></td><td>{display(row.target_type)}<small>{display(row.target_id)}</small></td>{compact ? null : <><td>{display(row.request_id)}</td><td className="admin-structured-cell"><details><summary>View details</summary><StructuredValue value={row.details ?? {}} emptyLabel="No details" /></details></td></>}</tr>)}</DataTable>;
}

function Security({ policy, reload }: { policy: JsonRecord; reload: () => Promise<void> }) {
  const [mode, setMode] = useState<"disabled" | "invite_only" | "open">((policy.registration_mode as "disabled" | "invite_only" | "open") ?? "invite_only");
  const [message, setMessage] = useState(""); const [saving, setSaving] = useState(false);
  async function save() { setSaving(true); setMessage(""); try { await updateRegistrationPolicy(mode); await reload(); setMessage("Registration policy saved."); } catch (reason) { setMessage(reason instanceof Error ? reason.message : "Policy update failed"); } finally { setSaving(false); } }
  return <section className="admin-panel admin-policy"><PanelHeading title="Account registration" description="Invite-only is recommended for governed research deployments." /><label>Registration mode<select value={mode} onChange={(event) => setMode(event.target.value as typeof mode)}><option value="invite_only">Invite only</option><option value="disabled">Disabled</option><option value="open">Open registration</option></select></label><p>{mode === "invite_only" ? "A valid one-time invitation is required." : mode === "disabled" ? "Only existing users can sign in." : "Anyone who can reach this service can register."}</p><div className="admin-policy-actions"><span role="status">{message}</span><button className="primary-button" disabled={saving} onClick={() => void save()}>{saving ? "Saving..." : "Save policy"}</button></div></section>;
}

function Metric({ label, value, supporting }: { label: string; value: unknown; supporting?: string }) { return <div className="admin-metric"><span>{label}</span><strong>{metricValue(value)}</strong>{supporting ? <small>{supporting}</small> : <small>Live control-plane count</small>}</div>; }
function PanelHeading({ title, description }: { title: string; description: string }) { return <header className="admin-panel-heading"><div><h2>{title}</h2><p>{description}</p></div></header>; }
function TableSearch({ value, onChange, label }: { value: string; onChange: (value: string) => void; label: string }) { return <label className="admin-table-search"><Search /><span className="sr-only">{label}</span><input value={value} onChange={(event) => onChange(event.target.value)} placeholder={label} /></label>; }
function DataTable({ headings, children, empty }: { headings: string[]; children: React.ReactNode; empty: string }) { const count = Array.isArray(children) ? children.length : children ? 1 : 0; return <div className="admin-table-wrap"><table className="admin-data-table"><thead><tr>{headings.map((heading) => <th key={heading}>{heading}</th>)}</tr></thead><tbody>{children}</tbody></table>{count === 0 ? <div className="admin-table-empty"><strong>No data</strong><span>{empty}</span></div> : null}</div>; }
function Unavailable({ title, description }: { title: string; description: string }) { return <section className="admin-unavailable"><strong>{title}</strong><p>{description}</p></section>; }
function useSearch(rows: JsonRecord[], query: string) { return useMemo(() => { const needle = query.trim().toLowerCase(); return needle ? rows.filter((row) => Object.values(row).some((value) => String(value ?? "").toLowerCase().includes(needle))) : rows; }, [query, rows]); }
function display(value: unknown) { return value === null || value === undefined || value === "" ? "Unavailable" : String(value); }
function metricValue(value: unknown) { return typeof value === "number" && Number.isFinite(value) ? value.toLocaleString() : "Unavailable"; }
function metricSupport(value: unknown, label: string) { return typeof value === "number" ? `${value.toLocaleString()} ${label}` : undefined; }
function formatDate(value: unknown) { if (typeof value !== "string" || !value) return "Unavailable"; const date = new Date(value); return Number.isNaN(date.getTime()) ? value : date.toLocaleString(); }
