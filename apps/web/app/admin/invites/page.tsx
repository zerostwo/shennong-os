"use client";

import { FormEvent, useCallback, useEffect, useState } from "react";
import { Check, Copy, TicketCheck, Trash2 } from "lucide-react";
import { AppShell } from "@/components/app-shell";
import {
  createRegistrationInvite,
  listRegistrationInvites,
  revokeRegistrationInvite,
  type RegistrationInviteRecord,
} from "@/lib/api/adapter";

export default function InvitationsPage() {
  const [invites, setInvites] = useState<RegistrationInviteRecord[]>([]);
  const [plaintextCode, setPlaintextCode] = useState("");
  const [copied, setCopied] = useState(false);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const load = useCallback(async () => {
    try { setInvites(await listRegistrationInvites()); }
    catch (reason) { setError(reason instanceof Error ? reason.message : "Invitations could not be loaded"); }
  }, []);
  useEffect(() => { void load(); }, [load]);

  async function create(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setBusy(true);
    setError("");
    const form = new FormData(event.currentTarget);
    try {
      const durationDays = Math.max(1, Math.min(90, Number(form.get("duration_days") ?? 7)));
      const result = await createRegistrationInvite({
        email_constraint: String(form.get("email_constraint") ?? "").trim() || undefined,
        max_uses: Math.max(1, Math.min(100, Number(form.get("max_uses") ?? 1))),
        expires_at: new Date(Date.now() + durationDays * 86_400_000).toISOString(),
        note: String(form.get("note") ?? "").trim() || undefined,
      });
      setPlaintextCode(result.code);
      setCopied(false);
      event.currentTarget.reset();
      await load();
    } catch (reason) { setError(reason instanceof Error ? reason.message : "Invitation could not be created"); }
    finally { setBusy(false); }
  }

  return (
    <AppShell active="admin" variant="admin">
      <div className="admin-page">
        <header className="page-header"><div><h1>Invitations</h1><p>Issue, audit, and revoke early-access registration codes.</p></div></header>
        {error ? <p className="form-error" role="alert">{error}</p> : null}
        {plaintextCode ? <section className="invite-once"><TicketCheck /><div><strong>Copy this code now</strong><p>Only its cryptographic digest is stored. It will not be shown again.</p><code>{plaintextCode}</code></div><button onClick={() => void navigator.clipboard.writeText(plaintextCode).then(() => setCopied(true))}>{copied ? <Check /> : <Copy />}{copied ? "Copied" : "Copy"}</button></section> : null}
        <section className="panel"><h2>Create invitation</h2><form className="invite-form" onSubmit={(event) => void create(event)}>
          <label>Email constraint (optional)<input name="email_constraint" type="email" placeholder="researcher@example.org" /></label>
          <label>Maximum uses<input name="max_uses" type="number" min="1" max="100" defaultValue="1" required /></label>
          <label>Valid days<input name="duration_days" type="number" min="1" max="90" defaultValue="7" required /></label>
          <label>Note<input name="note" maxLength={240} /></label>
          <button className="primary-button" disabled={busy}>{busy ? "Creating…" : "Create invitation"}</button>
        </form></section>
        <section className="panel"><h2>Issued invitations</h2><div className="table-wrap"><table><thead><tr><th>Prefix</th><th>Email</th><th>Uses</th><th>Expires</th><th>Status</th><th /></tr></thead><tbody>
          {invites.map((invite) => <tr key={invite.id}><td><code>{invite.codePrefix}…</code></td><td>{invite.emailConstraint ?? "Any invited user"}</td><td>{invite.useCount} / {invite.maxUses}</td><td>{invite.expiresAt ? new Date(invite.expiresAt).toLocaleString() : "Not available"}</td><td>{invite.revokedAt ? "Revoked" : new Date(invite.expiresAt).getTime() < Date.now() ? "Expired" : "Active"}</td><td>{!invite.revokedAt ? <button className="icon-button" aria-label="Revoke invitation" onClick={() => void revokeRegistrationInvite(invite.id).then(load)}><Trash2 /></button> : null}</td></tr>)}
          {!invites.length ? <tr><td colSpan={6}>No invitations have been issued.</td></tr> : null}
        </tbody></table></div></section>
      </div>
    </AppShell>
  );
}
