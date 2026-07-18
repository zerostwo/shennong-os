"use client";

import { useCallback, useEffect, useState } from "react";
import Link from "next/link";
import { ArrowLeft, ShieldCheck, X } from "lucide-react";
import { getUser, updateUser, type JsonRecord } from "@/lib/api/adapter";
import { AppShell, TinyBadge, TopBar } from "./app-shell";
import { StructuredValue } from "./structured-value";

export function UserDetailView({ userId }: { userId: string }) {
  const [user, setUser] = useState<JsonRecord | null>(null);
  const [action, setAction] = useState<"role" | "status" | null>(null);
  const [error, setError] = useState("");
  const load = useCallback(async () => { setError(""); try { setUser(await getUser(userId)); } catch (reason) { setError(reason instanceof Error ? reason.message : "User could not be loaded"); } }, [userId]);
  useEffect(() => { void load(); }, [load]);
  async function save(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault(); if (!user) return;
    const form = new FormData(event.currentTarget);
    const nextRole = String(form.get("role") ?? user.role);
    const nextStatus = String(form.get("status") ?? user.status);
    try {
      await updateUser({ id: userId, display_name: String(user.display_name), email: String(user.email), role: nextRole, status: nextStatus });
      setAction(null); await load();
    } catch (reason) { setError(reason instanceof Error ? reason.message : "User update failed"); }
  }
  return <AppShell variant="admin" active="users">
    <TopBar title={user ? String(user.display_name) : "User detail"} description={user ? String(user.email) : `User ${userId}`} search={false} action={<Link href="/admin/users" className="outline-button"><ArrowLeft />Users</Link>} />
    <div className="admin-page">
      {error ? <div className="error-banner" role="alert">{error}</div> : null}
      {!user && !error ? <div className="admin-loading"><span /><span /><span /></div> : null}
      {user ? <section className="admin-panel user-detail">
        <div className="user-detail-header"><span className="avatar avatar-green">{String(user.display_name).slice(0, 2).toUpperCase()}</span><div><h2>{String(user.display_name)}</h2><p>{String(user.email)}</p><div className="tag-row"><TinyBadge tone={user.role === "admin" ? "purple" : "neutral"}>{String(user.role)}</TinyBadge><TinyBadge tone={user.status === "active" ? "green" : "amber"}>{String(user.status)}</TinyBadge></div></div><div className="user-actions"><button className="outline-button" onClick={() => setAction("role")}>Change role</button><button className="danger-button" onClick={() => setAction("status")}>{user.status === "active" ? "Disable account" : "Enable account"}</button></div></div>
        <div className="user-detail-body"><div><h3>Identity and usage</h3><StructuredValue value={{ id: user.id, email: user.email, display_name: user.display_name, role: user.role, status: user.status, owned_projects: user.owned_projects, active_sessions: user.active_sessions, enabled_model_providers: user.enabled_providers, created_at: user.created_at, updated_at: user.updated_at }} /></div><aside className="admin-context-note"><ShieldCheck /><div><strong>Administrative boundary</strong><p>This view exposes identity metadata and aggregate ownership counts. Session contents, passwords, and provider secrets are never returned.</p></div></aside></div>
      </section> : null}
    </div>
    {action && user ? <div className="modal-scrim"><form className="simple-dialog" onSubmit={(event) => void save(event)}><button type="button" className="dialog-close" onClick={() => setAction(null)} aria-label="Close dialog"><X /></button><h2>{action === "role" ? "Change user role" : "Change account status"}</h2>{action === "role" ? <label>Role<select name="role" defaultValue={String(user.role)}><option value="user">User</option><option value="admin">Administrator</option></select></label> : <><p>{user.status === "active" ? "Disabling this account revokes its active sessions." : "Enabling this account allows sign-in again."}</p><input type="hidden" name="status" value={user.status === "active" ? "disabled" : "active"} /></>}<div className="dialog-actions"><button type="button" className="outline-button" onClick={() => setAction(null)}>Cancel</button><button className={action === "status" ? "danger-button" : "primary-button"}>Confirm change</button></div></form></div> : null}
  </AppShell>;
}
