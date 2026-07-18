"use client";

import { useEffect, useState } from "react";
import { KeyRound, LogOut } from "lucide-react";
import { listSessions, revokeSession, type JsonRecord } from "@/lib/api/adapter";
import { SectionHeader } from "./app-shell";

type Page = "profile" | "security" | "sessions" | "login-history";

export function AccountView({ page }: { page: Page }) {
  if (page === "sessions") return <Sessions />;
  return null;
}

function Sessions() {
  const [rows, setRows] = useState<JsonRecord[]>([]);
  const [error, setError] = useState("");

  async function load() {
    setRows(await listSessions());
  }

  useEffect(() => {
    void load().catch((reason) => setError(reason instanceof Error ? reason.message : "Session request failed"));
  }, []);

  async function revoke(id: string) {
    try {
      await revokeSession(id);
      await load();
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "Revoke failed");
    }
  }

  return (
    <div className="console-panel">
      <SectionHeader title="Active sessions" description="Server-side session records backed by revocable access tokens." />
      {error ? <div className="error-banner" role="alert">{error}</div> : null}
      <div className="record-table-wrap">
        <table className="simple-table">
          <thead><tr><th>User agent</th><th>IP address</th><th>Created</th><th>Last seen</th><th>Expires</th><th>Action</th></tr></thead>
          <tbody>
            {rows.map((row) => (
              <tr key={text(row.token_id)}>
                <td><strong>{text(row.user_agent)}</strong></td>
                <td>{text(row.ip_address)}</td>
                <td>{text(row.created_at)}</td>
                <td>{text(row.last_seen_at)}</td>
                <td>{text(row.expires_at)}</td>
                <td><button className="danger-button compact" onClick={() => void revoke(text(row.token_id))}><LogOut />Revoke</button></td>
              </tr>
            ))}
          </tbody>
        </table>
        {rows.length === 0 ? <div className="empty-state"><KeyRound /><h3>No records</h3><p>The API returned no persisted records.</p></div> : null}
      </div>
    </div>
  );
}

function text(value: unknown, fallback = "—") {
  return value === null || value === undefined || value === "" ? fallback : String(value);
}
