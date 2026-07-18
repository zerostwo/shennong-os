"use client";

import Link from "next/link";
import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Activity, ArrowRight, Beaker, Boxes, Database, FlaskConical, FolderUp, GitBranch, MessageSquare, Network, ShieldAlert, SlidersHorizontal } from "lucide-react";
import {
  getProjectContextPack,
  listProjectActivities,
  listProjectEntities,
  listProjectResources,
  type JsonRecord,
  type ProjectContextPack,
} from "@/lib/api/adapter";
import { AppShell, TinyBadge, TopBar } from "./app-shell";
import { ProjectApiError } from "./projects-view";
import { ProjectObservationTable } from "./project-observation-table";
import { ProjectTabs } from "./project-tabs";
import styles from "./project-ui.module.css";

const projectKey = (projectId: string) => ["projects", projectId] as const;

export function ProjectWorkspaceView({ projectId }: { projectId: string }) {
  const [loadFullLists, setLoadFullLists] = useState(false);
  const context = useQuery({ queryKey: [...projectKey(projectId), "context-pack"], queryFn: () => getProjectContextPack(projectId) });
  const fullEntities = useQuery({ queryKey: [...projectKey(projectId), "entities"], queryFn: () => listProjectEntities(projectId), enabled: context.data?.truncated === true && loadFullLists });
  const fullActivities = useQuery({ queryKey: [...projectKey(projectId), "activities"], queryFn: () => listProjectActivities(projectId), enabled: context.data?.truncated === true && loadFullLists });
  const fullResources = useQuery({ queryKey: [...projectKey(projectId), "resources"], queryFn: () => listProjectResources(projectId), enabled: context.data?.truncated === true && loadFullLists });
  const entities = fullEntities.data ?? context.data?.entities;
  const activities = fullActivities.data ?? context.data?.activities;
  const resources = fullResources.data ?? context.data?.resources;

  return (
    <AppShell active="projects">
      <TopBar
        title={context.data?.project.name ?? "Project"}
        description={context.data?.project.description || `Research workspace ${projectId}`}
        search={false}
        action={<Link className="primary-button" href={`/projects/${encodeURIComponent(projectId)}/chat`}><MessageSquare />Chat with Agent</Link>}
      />
      <div className="workspace-page project-workspace-page">
        <ProjectTabs projectId={projectId} active="workspace" />
        {context.error ? <ProjectApiError error={context.error} /> : null}
        {context.isPending ? <div className="loading-state">Loading project…</div> : null}
        {context.data ? (
          <>
            <section className={styles.workspaceSummary} aria-label="Project next steps and status">
              <div className={styles.primaryFlow}>
                <h2>Continue your research</h2>
                <p>Work with the Agent to add data, plan analyses, and keep the Project organized.</p>
                <Link className={styles.primaryAction} href={`/projects/${encodeURIComponent(projectId)}/chat`}>
                  <MessageSquare />
                  <span><strong>Open Project chat</strong><small>Ask a question or attach experimental files for the Agent to organize.</small></span>
                  <ArrowRight />
                </Link>
                <div className={styles.secondaryActions}>
                  <Link href={`/projects/${encodeURIComponent(projectId)}/uploads/new`}><FolderUp />Add data</Link>
                  <Link href={`/projects/${encodeURIComponent(projectId)}/graph`}><Network />Explore BioGraph</Link>
                  <Link href={`/projects/${encodeURIComponent(projectId)}/compute`}><SlidersHorizontal />Compute</Link>
                </div>
              </div>
              <div className={styles.statusPanel}>
                <h2>Research status</h2>
                <p>A compact view of the persisted Project boundary.</p>
                <div className={styles.statusLine}><strong>{context.data.project.status || "active"}</strong><TinyBadge tone={context.data.project.visibility === "private" ? "amber" : "green"}>{context.data.project.visibility}</TinyBadge></div>
                <div className={styles.metricStrip}>
                  <Metric label="Studies" value={context.data.studies.length} />
                  <Metric label="Entities" value={entities?.length} />
                  <Metric label="Activities" value={activities?.length} />
                  <Metric label="Resources" value={resources?.length} />
                </div>
              </div>
            </section>

            {context.data.truncated ? (
              <div className="project-limit-notice" role="status"><ShieldAlert /><span><strong>Some Project records are not shown.</strong> Load the complete lists when you need technical detail.</span><button className="outline-button" disabled={loadFullLists} onClick={() => setLoadFullLists(true)}>{loadFullLists ? "Loading full lists…" : "Load complete lists"}</button></div>
            ) : null}

            <section className={styles.contentSection}>
              <header><div><h2>Research contents</h2><p>Studies and versioned resources available to the Agent in this Project.</p></div></header>
              <div className={styles.contentGrid}>
                <ContentGroup title="Studies"><StudyList rows={context.data.studies} /></ContentGroup>
                <ContentGroup title="Resources">
                  {fullResources.error ? <ProjectApiError error={fullResources.error} compact /> : null}
                  <ResourceList resources={resources ?? []} />
                </ContentGroup>
              </div>
            </section>

            <details className={styles.technicalDetails}>
              <summary><GitBranch />Technical Project details</summary>
              <div className={styles.technicalBody}>
                <ContextPackDetails context={context.data} />
                <ProjectObservationTable projectId={projectId} />
                <TechnicalRecords entities={entities ?? []} activities={activities ?? []} />
              </div>
            </details>
          </>
        ) : null}
      </div>
    </AppShell>
  );
}

function Metric({ label, value }: { label: string; value: number | undefined }) {
  return <div className={styles.metric}><small>{label}</small><strong>{value ?? "…"}</strong></div>;
}

function ContentGroup({ title, children }: { title: string; children: React.ReactNode }) {
  return <div className={styles.contentGroup}><h3>{title}</h3>{children}</div>;
}

function ResourceList({ resources }: { resources: ProjectContextPack["resources"] }) {
  if (!resources.length) return <ProjectEmpty label="No resources yet. Add files in Project chat." />;
  return <div className={styles.rowList}>{resources.slice(0, 6).map((resource) => (
    <Link key={resource.id} href={`/resources/${encodeURIComponent(resource.id)}`}><Boxes /><span><strong>{resource.name}</strong><small>{resource.backend || resource.id}</small></span><ArrowRight /></Link>
  ))}</div>;
}

function StudyList({ rows }: { rows: JsonRecord[] }) {
  if (!rows.length) return <ProjectEmpty label="No study records yet." />;
  return <div className={styles.rowList}>{rows.slice(0, 6).map((study, index) => {
    const id = text(study.id, `study-${index + 1}`);
    return <div key={id}><Beaker /><span><strong>{text(study.label ?? study.name, id)}</strong><small>{text(study.description, text(study.kind, "Study"))}</small></span></div>;
  })}</div>;
}

function ContextPackDetails({ context }: { context: ProjectContextPack }) {
  return <section className={styles.technicalGroup}><h3>Agent context coverage</h3><div className={styles.technicalFacts}>
    <div><strong>Research graph</strong><p>{context.associations.length} associations and {context.evidence.length} evidence items.</p></div>
    <div><strong>Execution provenance</strong><p>{context.activityIo.length} input/output links and {context.activityActors.length} actor assignments.</p></div>
    <div><strong>Data products</strong><p>{context.resourceRevisions.length} immutable revisions and {context.resourceGraphBindings.length} graph bindings.</p></div>
  </div></section>;
}

function TechnicalRecords({ entities, activities }: { entities: ProjectContextPack["entities"]; activities: ProjectContextPack["activities"] }) {
  return <section className={styles.technicalGroup}><h3>Recent persisted records</h3><div className={styles.contentGrid}>
    <ContentGroup title="Entities"><div className={styles.rowList}>{entities.slice(0, 8).map((entity) => <div key={entity.id}><Database /><span><strong>{entity.label}</strong><small>{entity.category} · {entity.kind}</small></span><StateBadge state={entity.state} /></div>)}{!entities.length ? <ProjectEmpty label="No entities." /> : null}</div></ContentGroup>
    <ContentGroup title="Activities"><div className={styles.rowList}>{activities.slice(0, 8).map((activity) => <div key={activity.id}><Activity /><span><strong>{activity.label}</strong><small>{activity.kind} · {formatDate(activity.startedAt)}</small></span><StateBadge state={activity.status} /></div>)}{!activities.length ? <ProjectEmpty label="No activities." /> : null}</div></ContentGroup>
  </div></section>;
}

function StateBadge({ state }: { state: string }) {
  const normalized = state.toLowerCase();
  const tone = normalized === "validated" || normalized === "completed" || normalized === "available" ? "green" : normalized === "hypothesis" || normalized === "proposed" ? "amber" : normalized === "computed" ? "purple" : "blue";
  return <TinyBadge tone={tone}>{state || "unknown"}</TinyBadge>;
}

function ProjectEmpty({ label }: { label: string }) {
  return <div className={styles.emptyRow}><FlaskConical />{label}</div>;
}

function text(value: unknown, fallback = "Not available") {
  return typeof value === "string" && value.length > 0 ? value : fallback;
}

function formatDate(value: string) {
  if (!value) return "No date";
  const parsed = new Date(value);
  return Number.isNaN(parsed.getTime()) ? value : parsed.toLocaleDateString();
}
