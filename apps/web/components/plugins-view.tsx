"use client";

import { useEffect, useState } from "react";
import { Boxes, CheckCircle2, Puzzle, ServerOff } from "lucide-react";
import { AppShell } from "@/components/app-shell";
import { SkillsSettings } from "@/components/settings-dialog";
import { getCapabilities, getSession, type JsonRecord } from "@/lib/api/adapter";

type Tab = "plugins" | "skills";

export function PluginsView() {
  const [tab, setTab] = useState<Tab>("plugins");
  const [authenticated, setAuthenticated] = useState(false);
  const [capabilities, setCapabilities] = useState<JsonRecord | null>(null);
  useEffect(() => {
    void Promise.all([getSession(), getCapabilities()]).then(([session, current]) => {
      setAuthenticated(session.authenticated);
      setCapabilities(current);
    }).catch(() => setCapabilities({}));
  }, []);
  return (
    <AppShell active="plugins">
      <div className="plugins-page">
        <header className="plugins-heading">
          <div><h1>Plugins</h1><p>Manage Agent runtime extensions and versioned Skills from one place.</p></div>
          <div className="plugins-tabs" role="tablist" aria-label="Plugin workspace">
            <button role="tab" aria-selected={tab === "plugins"} className={tab === "plugins" ? "active" : ""} onClick={() => setTab("plugins")}><Boxes />Plugins</button>
            <button role="tab" aria-selected={tab === "skills"} className={tab === "skills" ? "active" : ""} onClick={() => setTab("skills")}><Puzzle />Skills</button>
          </div>
        </header>
        {tab === "plugins" ? <RuntimePlugins capabilities={capabilities} /> : <SkillsSettings authenticated={authenticated} standalone />}
      </div>
    </AppShell>
  );
}

function RuntimePlugins({ capabilities }: { capabilities: JsonRecord | null }) {
  const connected = capabilities?.agent_gateway === true;
  return (
    <section className="runtime-plugins" role="tabpanel">
      <header><h2>Pi Agent plugins</h2><p>Plugins run inside the Pi Agent host. This page only reports capabilities exposed by the Shennong server.</p></header>
      <div className="runtime-plugin-row">
        <span className="runtime-plugin-icon">{connected ? <CheckCircle2 /> : <ServerOff />}</span>
        <span><strong>Shennong Agent runtime</strong><small>{capabilities === null ? "Checking runtime capability" : connected ? "Connected through the governed Agent gateway" : "Agent gateway is not available"}</small></span>
        <span className={`runtime-plugin-status ${connected ? "available" : "unavailable"}`}>{connected ? "Available" : "Unavailable"}</span>
      </div>
      <div className="plugin-registry-note"><strong>No installable plugin registry is exposed</strong><p>Pi Agent extensions must currently be installed and configured on the Agent host. Shennong does not claim or simulate integrations that the runtime has not reported.</p></div>
    </section>
  );
}
