"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { FormEvent, useEffect, useMemo, useState } from "react";
import { useMutation, useQuery } from "@tanstack/react-query";
import { ArrowRight, CalendarClock, FolderKanban, Grid2X2, List, Plus, Search } from "lucide-react";
import {
  createProject,
  listProjects,
  ShennongApiError,
  type ProjectRecord,
} from "@/lib/api/adapter";
import { AppShell, EmptyState, SectionHeader, TinyBadge, TopBar } from "./app-shell";
import styles from "./project-ui.module.css";

const projectKeys = { all: ["projects"] as const };

export function ProjectsView() {
  const router = useRouter();
  const [query, setQuery] = useState("");
  const [view, setView] = useState<"list" | "grid">("list");
  const [creating, setCreating] = useState(false);
  const projects = useQuery({ queryKey: projectKeys.all, queryFn: listProjects });
  const createMutation = useMutation({
    mutationFn: createProject,
    onSuccess: (project) => {
      setCreating(false);
      router.push(`/projects/${encodeURIComponent(project.id)}`);
    },
  });
  const visibleProjects = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (!needle) return projects.data ?? [];
    return (projects.data ?? []).filter((project) =>
      `${project.name} ${project.description} ${project.ownerUserId}`.toLowerCase().includes(needle),
    );
  }, [projects.data, query]);
  useEffect(() => {
    const saved = window.localStorage.getItem("shennong-project-view");
    if (saved === "grid" || saved === "list") setView(saved);
  }, []);
  function changeView(next: "list" | "grid") {
    setView(next);
    window.localStorage.setItem("shennong-project-view", next);
  }

  function submitProject(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const form = new FormData(event.currentTarget);
    createMutation.mutate({
      name: String(form.get("name") ?? "").trim(),
      description: String(form.get("description") ?? "").trim(),
      visibility: form.get("visibility") === "public" ? "public" : "private",
    });
  }

  const error = projects.error ?? createMutation.error;
  return (
    <AppShell active="projects">
      <TopBar
        title="Projects"
        description="Private research workspaces that connect studies, experiments, analyses, and BioGraph evidence."
        search={false}
        action={<button className="primary-button" onClick={() => setCreating(true)}><Plus />New project</button>}
      />
      <div className="workspace-page projects-page">
        <SectionHeader
          title="Research projects"
          description="Every project is persisted by Shennong OS; this page does not create browser-local records."
        />
        <div className="workspace-toolbar projects-toolbar">
          <label className="filter-search">
            <Search />
            <input
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder="Search projects…"
            />
          </label>
          <div className={styles.viewToggle} role="group" aria-label="Project view">
            <button type="button" aria-label="List view" aria-pressed={view === "list"} onClick={() => changeView("list")}><List /></button>
            <button type="button" aria-label="Grid view" aria-pressed={view === "grid"} onClick={() => changeView("grid")}><Grid2X2 /></button>
          </div>
        </div>
        {error ? <ProjectApiError error={error} /> : null}
        {projects.isPending ? <div className="loading-state">Loading projects from Shennong OS…</div> : null}
        {!projects.isPending && !error && visibleProjects.length === 0 ? (
          <EmptyState
            title={query ? "No matching projects" : "No projects yet"}
            description={query ? "The live API returned no project matching this search." : "Create the first API-backed research workspace."}
            action={!query ? <button className="primary-button" onClick={() => setCreating(true)}><Plus />New project</button> : undefined}
          />
        ) : null}
        {visibleProjects.length > 0 ? (
          <div className={styles.projectCollection} data-view={view}>
            {visibleProjects.map((project) => <ProjectCard key={project.id} project={project} />)}
          </div>
        ) : null}
      </div>
      {creating ? (
        <div className="modal-scrim" role="presentation" onMouseDown={() => setCreating(false)}>
          <form className="simple-dialog project-dialog" onSubmit={submitProject} onMouseDown={(event) => event.stopPropagation()}>
            <div className="dialog-icon"><FolderKanban /></div>
            <h2>Create project</h2>
            <p>The project becomes the permission and provenance boundary for uploaded and Agent-acquired data.</p>
            {createMutation.error ? <ProjectApiError error={createMutation.error} compact /> : null}
            <label>Project name<input name="name" required minLength={3} autoFocus /></label>
            <label>Description<textarea name="description" rows={4} /></label>
            <label>Visibility<select name="visibility" defaultValue="private"><option value="private">Private</option><option value="public">Public</option></select></label>
            <div className="dialog-actions">
              <button type="button" className="outline-button" onClick={() => setCreating(false)}>Cancel</button>
              <button className="primary-button" disabled={createMutation.isPending}>{createMutation.isPending ? "Creating…" : "Create project"}</button>
            </div>
          </form>
        </div>
      ) : null}
    </AppShell>
  );
}

function ProjectCard({ project }: { project: ProjectRecord }) {
  return (
    <article className={styles.projectItem}>
      <div className={styles.projectIdentity}>
        <span><FolderKanban /></span>
        <div>
          <h2>{project.name}</h2>
          <code>{project.id}</code>
        </div>
      </div>
      <p className={styles.projectDescription}>{project.description || "No description provided."}</p>
      <div className={styles.projectMeta} aria-label="Project summary">
        <TinyBadge tone={project.visibility === "private" ? "amber" : "green"}>{project.visibility}</TinyBadge>
        <span><FolderKanban />{count(project.counts.resources)} resources</span>
        <span><CalendarClock />{formatDate(project.updatedAt || project.createdAt)}</span>
      </div>
      <Link className={styles.projectAction} href={`/projects/${encodeURIComponent(project.id)}`}>Open project<ArrowRight /></Link>
    </article>
  );
}

export function ProjectApiError({ error, compact = false }: { error: unknown; compact?: boolean }) {
  const notSupported = error instanceof ShennongApiError && error.code === "not_supported";
  const message = notSupported
    ? "This Shennong OS instance has not enabled the Projects API. No local placeholder data was created."
    : error instanceof Error
      ? error.message
      : "The Projects API request failed.";
  return <div className={`error-banner ${compact ? "compact" : ""}`} role="alert">{message}</div>;
}

function count(value: number | undefined) {
  return value === undefined ? "Not available" : String(value);
}

function formatDate(value: string) {
  if (!value) return "No update timestamp";
  const parsed = new Date(value);
  return Number.isNaN(parsed.getTime()) ? value : parsed.toLocaleDateString();
}
