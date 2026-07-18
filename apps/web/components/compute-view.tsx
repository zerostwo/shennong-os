"use client";

import { useCallback, useEffect, useState } from "react";
import { AppShell, TopBar } from "@/components/app-shell";
import { ProjectTabs } from "@/components/project-tabs";
import {
  cancelRuntimeJob,
  launchRuntimeSession,
  listRuntimeJobs,
  listRuntimeSessions,
  startRuntimeSession,
  stopRuntimeSession,
  type RuntimeJobRecord,
  type RuntimeSessionRecord,
} from "@/lib/api/adapter";
import { Clock3, Code2, ExternalLink, LoaderCircle, NotebookTabs, OctagonX, RefreshCw, SquareTerminal } from "lucide-react";

export function ComputeView({ projectId }: { projectId?: string }) {
  const [jobs, setJobs] = useState<RuntimeJobRecord[]>([]);
  const [sessions, setSessions] = useState<RuntimeSessionRecord[]>([]);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState("");
  const [error, setError] = useState("");

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const [jobRows, sessionRows] = await Promise.all([
        listRuntimeJobs(projectId),
        projectId ? listRuntimeSessions(projectId) : Promise.resolve([]),
      ]);
      setJobs(jobRows);
      setSessions(sessionRows);
      setError("");
    } catch (reason) { setError(reason instanceof Error ? reason.message : "Compute state could not be loaded"); }
    finally { setLoading(false); }
  }, [projectId]);
  useEffect(() => { void load(); }, [load]);

  async function start(kind: RuntimeSessionRecord["kind"]) {
    if (!projectId) return;
    setBusy(kind);
    try { await startRuntimeSession(projectId, kind); await load(); }
    catch (reason) { setError(reason instanceof Error ? reason.message : "Session could not be started"); }
    finally { setBusy(""); }
  }

  async function openSession(session: RuntimeSessionRecord) {
    const ideWindow = window.open("about:blank", "_blank");
    if (!ideWindow) {
      setError("The browser blocked the IDE window. Allow pop-ups for Shennong and try again.");
      return;
    }
    ideWindow.opener = null;
    setBusy(`open:${session.id}`);
    setError("");
    try {
      const launch = await launchRuntimeSession(session.id);
      // replace() keeps the one-time ticket out of React state and avoids
      // retaining the launch URL as a separate browser-history entry.
      ideWindow.location.replace(launch.launchUrl);
    } catch {
      ideWindow.close();
      setError("The IDE could not be opened. Refresh the session state and try again.");
    } finally {
      setBusy("");
    }
  }

  const content = (
    <div className="compute-page">
      <header className="page-header"><div><h1>Compute</h1><p>Isolated analysis jobs and interactive project workspaces.</p></div><button className="outline-button" onClick={() => void load()}><RefreshCw />Refresh</button></header>
      {error ? <p className="form-error" role="alert">{error}</p> : null}
      {projectId ? <section className="compute-launchers">
        <button onClick={() => void start("rstudio")} disabled={Boolean(busy)}><SquareTerminal /><span><strong>RStudio Server</strong><small>Open the project in an isolated R workspace.</small></span>{busy === "rstudio" ? <LoaderCircle className="spin" /> : null}</button>
        <button onClick={() => void start("jupyterlab")} disabled={Boolean(busy)}><NotebookTabs /><span><strong>JupyterLab</strong><small>Use Python, R kernels, terminals, and notebooks.</small></span>{busy === "jupyterlab" ? <LoaderCircle className="spin" /> : null}</button>
      </section> : <p className="panel-note">Interactive RStudio and JupyterLab sessions are available inside a Project.</p>}
      {projectId ? <section className="panel"><h2>Interactive sessions</h2><div className="compute-list">
        {sessions.map((session) => {
          const label = session.kind === "rstudio" ? "RStudio Server" : "JupyterLab";
          const opening = busy === `open:${session.id}`;
          return <article key={session.id}><span className={`status-dot ${session.status}`} /><Code2 /><div><strong>{label}</strong><small>{session.status} · {session.expiresAt ? `expires ${new Date(session.expiresAt).toLocaleString()}` : "managed lifetime"}</small></div>{session.status === "running" ? <button className="outline-button" aria-label={`Open ${label}`} disabled={Boolean(busy)} onClick={() => void openSession(session)}>{opening ? "Opening…" : "Open"} {opening ? <LoaderCircle className="spin" /> : <ExternalLink />}</button> : null}{session.status === "running" ? <button className="icon-button" aria-label={`Stop ${label}`} onClick={() => void stopRuntimeSession(session.id).then(load)}><OctagonX /></button> : null}</article>;
        })}
        {!sessions.length && !loading ? <p>No interactive sessions.</p> : null}
      </div></section> : null}
      <section className="panel"><h2>Analysis jobs</h2><div className="compute-list">
        {jobs.map((job) => <article key={job.id}><span className={`status-dot ${job.status}`} /><Clock3 /><div><strong>{job.id}</strong><small>{job.status} · {job.workerProfile}{job.exitCode !== undefined ? ` · exit ${job.exitCode}` : ""}</small></div>{["queued", "preparing", "running"].includes(job.status) ? <button className="outline-button" onClick={() => void cancelRuntimeJob(job.id).then(load)}>Cancel</button> : null}</article>)}
        {!jobs.length && !loading ? <p>No analysis jobs.</p> : null}
        {loading ? <p><LoaderCircle className="spin" />Loading compute state…</p> : null}
      </div></section>
    </div>
  );
  return projectId
    ? <AppShell active="projects"><TopBar title="Project Compute" description="Rootless, isolated analysis jobs and IDE sessions." search={false} /><div className="workspace-page project-workspace-page"><ProjectTabs projectId={projectId} active="compute" />{content}</div></AppShell>
    : <AppShell active="compute">{content}</AppShell>;
}
