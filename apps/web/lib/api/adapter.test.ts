import { afterEach, describe, expect, it, vi } from "vitest";
import {
  archiveAgentMemory,
  createAiProvider,
  createAgentSkill,
  createGuidedAgentSkillDraft,
  createProjectChatThread,
  createProjectMemory,
  deleteAiProvider,
  enableThreadSkill,
  getBioGraphSubgraph,
  getChatThread,
  getProjectContextPack,
  getResource,
  getSession,
  listAiProviders,
  listAgentSkills,
  listChatThreads,
  listThreadSkills,
  listGlobalMemories,
  listProjectMemories,
  listProjectChatThreads,
  listProjectUploads,
  listProjects,
  listResources,
  launchRuntimeSession,
  registerUser,
  registerProjectUploads,
  searchWorkspace,
  sendChatMessage,
  signIn,
  setupAdmin,
  submitProjectObservations,
  updateAgentSkill,
  updateAgentMemory,
  updateAiProvider,
  uploadProjectFile,
} from "./adapter";

describe("OS authentication contract", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("normalizes the direct V1 sign-in session payload", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(jsonResponse({ data: {
      id: "admin-1",
      email: "admin@example.test",
      display_name: "Admin",
      role: "admin",
      csrf_token: "not-exposed-by-the-adapter",
    } })));

    await expect(signIn("admin@example.test", "correct-horse-battery-staple")).resolves.toEqual({
      authenticated: true,
      user_id: "admin-1",
      role: "admin",
      scopes: [],
    });
  });

  it("normalizes the nested V1 current-session payload", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(jsonResponse({ data: {
      authenticated: true,
      user: { id: "user-1", role: "user" },
    } })));

    await expect(getSession()).resolves.toMatchObject({
      authenticated: true,
      user_id: "user-1",
      role: "user",
      scopes: [],
    });
  });

  it("honors an explicit unauthenticated response even when stale identity fields are present", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(jsonResponse({ data: {
      authenticated: false,
      user: { id: "stale-user", role: "admin" },
    } })));

    await expect(getSession()).resolves.toMatchObject({
      authenticated: false,
      user_id: "",
      role: "",
      scopes: [],
    });
  });
});

describe("bootstrap setup", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("sends the one-time bootstrap token only as its dedicated header", async () => {
    const fetchMock = vi.fn().mockResolvedValue(jsonResponse({ data: { id: "admin-1", role: "admin" } }, 201));
    vi.stubGlobal("fetch", fetchMock);

    await expect(setupAdmin("V1 Admin", "admin@example.test", "correct-horse-battery-staple", "bootstrap-secret")).resolves.toMatchObject({
      authenticated: true,
      user_id: "admin-1",
      role: "admin",
    });

    expect(String(fetchMock.mock.calls[0][0])).toMatch(/\/setup\/admin$/);
    const init = fetchMock.mock.calls[0][1] as RequestInit;
    expect(new Headers(init.headers).get("x-shennong-bootstrap-token")).toBe("bootstrap-secret");
    expect(JSON.parse(String(init.body))).toEqual({
      display_name: "V1 Admin",
      email: "admin@example.test",
      password: "correct-horse-battery-staple",
    });
  });
});

describe("Project upload contract", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("uses only Project-scoped routes and never sends actor identity", async () => {
    const fetchMock = vi.fn()
      .mockResolvedValueOnce(jsonResponse({ data: { id: "upload-1" } }))
      .mockResolvedValueOnce(jsonResponse({ data: [{ id: "upload-1" }] }))
      .mockResolvedValueOnce(jsonResponse({ data: { id: "resource-1" } }));
    vi.stubGlobal("fetch", fetchMock);

    await uploadProjectFile(
      "project-1",
      new File(["gene\tvalue\n"], "matrix.tsv", {
        type: "text/tab-separated-values",
      }),
    );
    await listProjectUploads("project-1");
    await registerProjectUploads("project-1", {
      upload_ids: ["upload-1"],
      resource_id: "resource-1",
      name: "Expression matrix",
    });

    expect(fetchMock.mock.calls.map(([url]) => String(url))).toEqual([
      expect.stringMatching(/\/projects\/project-1\/uploads$/),
      expect.stringMatching(/\/projects\/project-1\/uploads$/),
      expect.stringMatching(/\/projects\/project-1\/uploads\/register$/),
    ]);
    const uploadHeaders = new Headers(fetchMock.mock.calls[0][1]?.headers);
    expect(uploadHeaders.get("x-filename")).toBe("matrix.tsv");
    expect(uploadHeaders.has("x-shennong-os-actor-id")).toBe(false);
    expect(uploadHeaders.has("x-shennong-os-project-id")).toBe(false);
    expect(JSON.parse(String(fetchMock.mock.calls[2][1]?.body))).not.toHaveProperty(
      "project_id",
    );
  });
});

describe("isolated IDE launch contract", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("mints a launch ticket with POST and returns only a validated IDE launch URL", async () => {
    const fetchMock = vi.fn().mockResolvedValue(jsonResponse({ data: {
      launch_url: "https://ide.example.test/__shennong/launch?ticket=one-time-ticket",
      expires_at: "2026-07-18T12:01:00Z",
    } }));
    vi.stubGlobal("fetch", fetchMock);

    await expect(launchRuntimeSession("session-1")).resolves.toEqual({
      launchUrl: "https://ide.example.test/__shennong/launch?ticket=one-time-ticket",
      expiresAt: "2026-07-18T12:01:00Z",
    });
    expect(String(fetchMock.mock.calls[0][0])).toMatch(/\/sessions\/session-1\/launch$/);
    expect(fetchMock.mock.calls[0][1]?.method).toBe("POST");
    expect(fetchMock.mock.calls[0][1]?.body).toBeUndefined();
  });

  it("rejects a non-HTTP or structurally invalid launch target", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(jsonResponse({ data: {
      launch_url: "javascript:alert(1)?ticket=not-safe",
      expires_at: "2026-07-18T12:01:00Z",
    } })));

    await expect(launchRuntimeSession("session-1")).rejects.toMatchObject({
      code: "invalid_launch_response",
    });
  });
});

describe("listResources", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("returns only records supplied by the live API", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response(JSON.stringify({ data: [{ id: "live-1", kind: "Resource", metadata: { title: "Live" }, permissions: { visibility: "public" }, spec: {} }] }), { status: 200, headers: { "content-type": "application/json" } })));
    const result = await listResources();
    expect(result.source).toBe("live");
    expect(result.data.map(({ id }) => id)).toEqual(["live-1"]);
    expect(result.data[0].kind).toBe("Resource");
  });

  it("normalizes API failures without exposing server internals", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response(JSON.stringify({ code:"not_found", message:"Resource not found", request_id:"req-42" }), { status:404, headers:{"content-type":"application/json"} })));
    await expect(getResource("private-record")).rejects.toMatchObject({ code:"not_found", message:"Resource not found", requestId:"req-42", status:404 });
  });

  it("preserves a provider error string when the API uses the legacy error field", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response(JSON.stringify({ error: "DeepSeek rejected the model request" }), { status: 502, headers: { "content-type": "application/json" } })));
    await expect(getResource("provider-error")).rejects.toMatchObject({ message: "DeepSeek rejected the model request", status: 502 });
  });

  it("parses the Rust error envelope and preserves its request ID", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response(JSON.stringify({
      error: {
        code: "project_forbidden",
        message: "project access is not allowed",
        request_id: "018f4d2c-0000-7000-8000-000000000042",
        details: { project_id: "project-1" },
      },
    }), { status: 403, headers: { "content-type": "application/json" } })));

    await expect(getResource("private-record")).rejects.toMatchObject({
      code: "project_forbidden",
      message: "project access is not allowed",
      requestId: "018f4d2c-0000-7000-8000-000000000042",
      details: { project_id: "project-1" },
      status: 403,
    });
  });

  it("does not invent catalog records when the live API returns none", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response(JSON.stringify({ data:[] }), { status:200, headers:{"content-type":"application/json"} })));
    const result = await listResources();
    expect(result.data).toEqual([]);
  });
});

describe("Projects and BioGraph API", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("normalizes projects without browser-local fallback records", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(jsonResponse({ data: [{
      id: "project-1",
      name: "Tumor atlas",
      description: "Integrated dry and wet lab evidence",
      owner_user_id: "user-1",
      visibility: "private",
      status: "active",
      created_at: "2026-07-14T00:00:00Z",
      updated_at: "2026-07-14T01:00:00Z",
    }] })));
    const projects = await listProjects();
    expect(projects).toHaveLength(1);
    expect(projects[0]).toMatchObject({ id: "project-1", name: "Tumor atlas", visibility: "private", status: "active" });
  });

  it("maps the real context-pack arrays and does not invent summary fields", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(jsonResponse({ data: {
      project: { id: "project-1", name: "Project 1", description: "", owner_user_id: "user-1", visibility: "private", status: "active", metadata: {}, created_at: "now", updated_at: "now" },
      studies: [{ id: "study-1", name: "Study 1" }],
      entities: [{ id: "sample-1", category: "sample", kind: "tissue", label: "Sample 1", status: "active", metadata: {}, created_at: "now" }],
      activities: [{ id: "activity-1", kind: "assay", label: "qPCR", status: "completed", parameters: {}, created_at: "now" }],
      activity_io: [{ activity_id: "activity-1", entity_id: "sample-1", direction: "input", role: "sample" }],
      activity_actors: [],
      associations: [{ id: "association-1", subject_id: "sample-1", predicate: "shennong:measured_by", object_id: "observation-1", polarity: "neutral", knowledge_level: "observation", status: "validated", qualifiers: {} }],
      evidence: [{ id: "evidence-1", evidence_type: "direct_observation" }],
      association_evidence: [{ association_id: "association-1", evidence_id: "evidence-1", stance: "supporting" }],
      resources: [],
      project_resources: [],
      resource_revisions: [],
      resource_graph_bindings: [],
      truncated: true,
    } })));
    const context = await getProjectContextPack("project-1");
    expect(context.project.name).toBe("Project 1");
    expect(context.studies).toHaveLength(1);
    expect(context.entities[0]).toMatchObject({ id: "sample-1", category: "sample" });
    expect(context.activities[0]).toMatchObject({ id: "activity-1", status: "completed" });
    expect(context.associations[0]).toMatchObject({ state: "validated", polarity: "neutral" });
    expect(context.activityIo).toHaveLength(1);
    expect(context.associationEvidence).toHaveLength(1);
    expect(context.truncated).toBe(true);
    expect(context.raw).not.toHaveProperty("summary");
  });

  it("reads the bounded subgraph entities and associations contract", async () => {
    const fetchMock = vi.fn().mockResolvedValue(jsonResponse({ data: {
      root_entity_id: "sample-1",
      depth: 2,
      truncated: false,
      entities: [
        { id: "sample-1", category: "sample", kind: "tissue", label: "Sample 1", status: "active", metadata: {} },
        { id: "observation-1", category: "observation", kind: "ct_value", label: "Ct", status: "active", metadata: {} },
      ],
      associations: [{ id: "association-1", subject_id: "sample-1", predicate: "shennong:has_observation", object_id: "observation-1", polarity: "neutral", knowledge_level: "observation", status: "proposed", qualifiers: {} }],
    } }));
    vi.stubGlobal("fetch", fetchMock);
    const graph = await getBioGraphSubgraph("project-1", "sample-1", 2, 500);
    expect(graph).toMatchObject({ root: "sample-1", depth: 2, truncated: false });
    expect(graph.nodes[1].state).toBe("observed");
    expect(graph.edges[0]).toMatchObject({ subjectId: "sample-1", objectId: "observation-1", state: "observed" });
    expect(String(fetchMock.mock.calls[0][0])).toContain("/projects/project-1/graph/subgraph?");
    expect(String(fetchMock.mock.calls[0][0])).toContain("limit=80");
  });

  it("persists observation activity, IO, association, evidence, and evidence link as distinct API mutations", async () => {
    const requests: Array<{ url: string; method: string; body: Record<string, unknown> }> = [];
    vi.stubGlobal("fetch", vi.fn().mockImplementation(async (input: string | URL | Request, init?: RequestInit) => {
      const body = init?.body ? JSON.parse(String(init.body)) as Record<string, unknown> : {};
      const url = String(input);
      requests.push({ url, method: init?.method ?? "GET", body });
      return jsonResponse({ data: body });
    }));
    const report = await submitProjectObservations("project-1", [{ sampleEntityId: "sample-1", measurementType: "ct_value", value: 21.4, unit: "Ct" }]);
    expect(report.complete).toBe(true);
    expect(report.activityIo).toHaveLength(1);
    expect(report.associationEvidence).toHaveLength(1);
    expect(requests.map(({ url }) => url)).toEqual([
      expect.stringMatching(/\/projects\/project-1\/activities$/),
      expect.stringMatching(/\/projects\/project-1\/entities$/),
      expect.stringMatching(/\/projects\/project-1\/activities\/activity-.*\/io$/),
      expect.stringMatching(/\/projects\/project-1\/associations$/),
      expect.stringMatching(/\/projects\/project-1\/evidence$/),
      expect.stringMatching(/\/projects\/project-1\/associations\/association-.*\/evidence\/evidence-.*/),
    ]);
    expect(requests[0].body).toMatchObject({ project_id: "project-1", kind: "observation_capture", status: "completed", parameters: { row_count: 1 } });
    expect(requests[1].body).toMatchObject({ project_id: "project-1", category: "observation", kind: "ct_value", metadata: { sample_id: "sample-1", value: 21.4, unit: "Ct" } });
    expect(requests[2].body).toMatchObject({ direction: "output", role: "observation", ordinal: 0 });
    expect(requests[3].body).toMatchObject({ subject_id: "sample-1", predicate: "shennong:has_observation", knowledge_level: "observation", polarity: "neutral", status: "proposed" });
    expect(requests[3].body).not.toHaveProperty("evidence");
    expect(requests[4].body).toMatchObject({ evidence_type: "direct_observation", source_id: expect.stringMatching(/^activity-/), statistics: { value: 21.4, unit: "Ct" } });
    expect(requests[5].body).toMatchObject({ stance: "supporting" });
  });
});

describe("Agent-first API", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("maps provider metadata without exposing or inventing an API key", async () => {
    const fetchMock = vi.fn().mockResolvedValue(jsonResponse({ data: [{
      id: "provider-1",
      name: "Lab Ollama",
      provider_kind: "ollama",
      base_url: "http://host.docker.internal:11434/v1",
      model: "local-model",
      data_policy: "allow_private",
      enabled: true,
      is_default: true,
      has_api_key: false,
      updated_at: "now",
    }] }));
    vi.stubGlobal("fetch", fetchMock);
    const providers = await listAiProviders();
    expect(providers).toEqual([expect.objectContaining({ id: "provider-1", providerType: "ollama", model: "local-model", dataPolicy: "allow_private", isDefault: true, hasApiKey: false })]);
    expect(providers[0].raw).not.toHaveProperty("api_key");
    expect(String(fetchMock.mock.calls[0][0])).toMatch(/\/providers$/);
  });

  it("uses the V1 provider collection and PATCH item contract", async () => {
    const provider = { id: "provider-1", name: "Lab Ollama", provider_kind: "ollama", base_url: "http://host.docker.internal:11434/v1", model: "qwen3", enabled: true };
    const fetchMock = vi.fn()
      .mockResolvedValueOnce(jsonResponse({ data: provider }, 201))
      .mockResolvedValueOnce(jsonResponse({ data: { ...provider, model: "qwen3:latest" } }))
      .mockResolvedValueOnce(new Response(null, { status: 204 }));
    vi.stubGlobal("fetch", fetchMock);

    const created = await createAiProvider({ name: "Lab Ollama", provider_kind: "ollama", base_url: provider.base_url, model: "qwen3" });
    const updated = await updateAiProvider(created.id, { model: "qwen3:latest" });
    await deleteAiProvider(updated.id);

    expect(fetchMock.mock.calls.map(([url]) => String(url))).toEqual([
      expect.stringMatching(/\/providers$/),
      expect.stringMatching(/\/providers\/provider-1$/),
      expect.stringMatching(/\/providers\/provider-1$/),
    ]);
    expect(fetchMock.mock.calls.map(([, init]) => init?.method)).toEqual(["POST", "PATCH", "DELETE"]);
  });

  it("loads a thread and its persisted messages from their separate endpoints", async () => {
    const fetchMock = vi.fn()
      .mockResolvedValueOnce(jsonResponse({ data: { id: "chat-1", title: "TP53 analysis", provider_id: "provider-1" } }))
      .mockResolvedValueOnce(jsonResponse({ data: [{ id: "message-1", role: "user", content: "Inspect TP53", attachments: [], tool_events: [], citations: [], created_at: "now" }] }));
    vi.stubGlobal("fetch", fetchMock);
    const thread = await getChatThread("chat-1");
    expect(thread).toMatchObject({ id: "chat-1", title: "TP53 analysis", providerId: "provider-1" });
    expect(thread.messages).toEqual([expect.objectContaining({ id: "message-1", role: "user", content: "Inspect TP53" })]);
    expect(fetchMock.mock.calls.map(([url]) => String(url))).toEqual([
      expect.stringMatching(/\/threads\/chat-1$/),
      expect.stringMatching(/\/threads\/chat-1\/messages$/),
    ]);
  });

  it("lists threads from the V1 thread collection used by search fallback", async () => {
    const fetchMock = vi.fn().mockResolvedValue(jsonResponse({ data: [{ id: "chat-1", project_id: "project-1", title: "TP53 analysis" }] }));
    vi.stubGlobal("fetch", fetchMock);
    await expect(listChatThreads()).resolves.toEqual([expect.objectContaining({ id: "chat-1", projectId: "project-1" })]);
    expect(String(fetchMock.mock.calls[0][0])).toMatch(/\/threads$/);
  });

  it("persists a user message through the idempotent V1 thread message endpoint", async () => {
    const fetchMock = vi.fn().mockResolvedValue(jsonResponse({ data: {
      id: "message-2",
      role: "user",
      content_json: "Analyze YTHDF2",
      attachments: [],
      metadata: { provider_id: "provider-1", reasoning_effort: "high" },
      created_at: "now",
    } }));
    vi.stubGlobal("fetch", fetchMock);
    const message = await sendChatMessage("chat-1", { content: "Analyze YTHDF2", provider_id: "provider-1", reasoning_effort: "high" });
    expect(message).toMatchObject({ role: "user", content: "Analyze YTHDF2" });
    expect(String(fetchMock.mock.calls[0][0])).toMatch(/\/threads\/chat-1\/messages$/);
    expect(fetchMock.mock.calls[0][1]?.method).toBe("POST");
    expect(new Headers(fetchMock.mock.calls[0][1]?.headers).get("idempotency-key")).toMatch(/^message-/);
    expect(JSON.parse(String(fetchMock.mock.calls[0][1]?.body))).toMatchObject({ role: "user", content_json: "Analyze YTHDF2", attachments: [], metadata: { provider_id: "provider-1", reasoning_effort: "high" } });
  });

  it("normalizes persisted built-in, user, and generated Skills", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(jsonResponse({ data: [
      { id: "skill-built-in", owner_user_id: null, slug: "resource-research", name: "Resource research", trust_level: "builtin_signed", lifecycle: "active", version: 1, content: "Inspect before querying." },
      { id: "skill-generated", owner_user_id: "user-1", slug: "colon-cancer", name: "Colon cancer analysis", trust_level: "generated", lifecycle: "draft", version: 2, content: "Draft workflow" },
    ] })));
    const skills = await listAgentSkills();
    expect(skills).toEqual([
      expect.objectContaining({ id: "skill-built-in", sourceKind: "built_in", status: "active", isBuiltin: true, revision: 1 }),
      expect.objectContaining({ id: "skill-generated", sourceKind: "generated", status: "draft", isBuiltin: false, revision: 2 }),
    ]);
  });

  it("creates a transparent guided draft and enables an active Skill through the real OS APIs", async () => {
    const fetchMock = vi.fn()
      .mockResolvedValueOnce(jsonResponse({ data: { id: "skill-generated", owner_user_id: "user-1", slug: "colon-cancer", name: "Colon cancer", trust_level: "generated", lifecycle: "draft", version: 1, content: "Guided workflow" } }, 201))
      .mockResolvedValueOnce(jsonResponse({ data: {} }));
    vi.stubGlobal("fetch", fetchMock);
    const skill = await createGuidedAgentSkillDraft({ name: "Colon cancer", goal: "Analyze colon cancer expression", constraints: ["Cite Resources"] });
    await enableThreadSkill("chat-1", "skill-active");
    expect(skill).toMatchObject({ id: "skill-generated", sourceKind: "generated", status: "draft" });
    expect(JSON.parse(String(fetchMock.mock.calls[0][1]?.body))).toMatchObject({ name: "Colon cancer", trust_level: "generated", lifecycle: "draft", manifest: { generated_by: "shennong-os-web-guided-draft" } });
    expect(String(fetchMock.mock.calls[0][0])).toMatch(/\/skills$/);
    expect(fetchMock.mock.calls[1]).toEqual([expect.stringMatching(/\/threads\/chat-1\/skills\/skill-active$/), expect.objectContaining({ method: "PUT", body: "{}" })]);
  });

  it("creates user Skills as drafts and saves instruction edits as immutable versions", async () => {
    const fetchMock = vi.fn()
      .mockResolvedValueOnce(jsonResponse({ data: { id: "skill-user", owner_user_id: "user-1", slug: "qc", name: "QC", trust_level: "user", lifecycle: "draft", version: 1, content: "Inspect" } }, 201))
      .mockResolvedValueOnce(jsonResponse({ data: { skill_id: "skill-user", version: 2 } }, 201))
      .mockResolvedValueOnce(jsonResponse({ data: { id: "skill-user", owner_user_id: "user-1", slug: "qc", name: "QC updated", trust_level: "user", lifecycle: "active", version: 2, content: "Inspect and validate" } }));
    vi.stubGlobal("fetch", fetchMock);
    const created = await createAgentSkill({ name: "QC", content: "Inspect" });
    const updated = await updateAgentSkill(created.id, { name: "QC updated", status: "active", content: "Inspect and validate", change_note: "Add validation" });
    expect(created.status).toBe("draft");
    expect(updated).toMatchObject({ status: "active", revision: 2 });
    expect(fetchMock.mock.calls.map(([url]) => String(url))).toEqual([
      expect.stringMatching(/\/skills$/),
      expect.stringMatching(/\/skills\/skill-user\/versions$/),
      expect.stringMatching(/\/skills\/skill-user$/),
    ]);
    expect(fetchMock.mock.calls.map(([, init]) => init?.method)).toEqual(["POST", "POST", "PATCH"]);
    expect(JSON.parse(String(fetchMock.mock.calls[1][1]?.body))).toMatchObject({ content: "Inspect and validate", change_note: "Add validation" });
    expect(JSON.parse(String(fetchMock.mock.calls[2][1]?.body))).toMatchObject({ name: "QC updated", lifecycle: "active" });
  });

  it("lists actual thread Skill selections from the unprefixed thread API", async () => {
    const fetchMock = vi.fn().mockResolvedValue(jsonResponse({ data: [
      { id: "skill-active", owner_user_id: null, name: "Inspect input", slug: "inspect-input", trust_level: "builtin_signed", lifecycle: "active", version: 3, enabled: true, selected_version: 2 },
    ] }));
    vi.stubGlobal("fetch", fetchMock);
    await expect(listThreadSkills("thread-1")).resolves.toEqual([
      expect.objectContaining({ id: "skill-active", enabled: true, revision: 3, selectedVersion: 2 }),
    ]);
    expect(String(fetchMock.mock.calls[0][0])).toMatch(/\/threads\/thread-1\/skills$/);
  });

  it("keeps global and Project memories on their scoped endpoints", async () => {
    const fetchMock = vi.fn()
      .mockResolvedValueOnce(jsonResponse({ data: [{ id: "memory-global", project_id: null, title: "Global preference", source_kind: "manual", lifecycle: "active", version: 1, content: "Always cite Resources." }] }))
      .mockResolvedValueOnce(jsonResponse({ data: [{ id: "memory-project", project_id: "project-1", title: "Cohort design", source_kind: "manual", lifecycle: "active", version: 1, content: "Tumor versus adjacent normal." }] }))
      .mockResolvedValueOnce(jsonResponse({ data: { id: "memory-project", project_id: "project-1", title: "Cohort design", source_kind: "manual", lifecycle: "active", version: 1, content: "Tumor versus adjacent normal." } }, 201))
      .mockResolvedValueOnce(jsonResponse({ data: { id: "memory-project", project_id: "project-1", title: "Cohort design", source_kind: "manual", lifecycle: "archived", version: 1, content: "Tumor versus adjacent normal." } }))
      .mockResolvedValueOnce(new Response(null, { status: 204 }));
    vi.stubGlobal("fetch", fetchMock);
    const global = await listGlobalMemories();
    const projectRows = await listProjectMemories("project-1");
    const project = await createProjectMemory("project-1", { title: "Cohort design", content: "Tumor versus adjacent normal." });
    const archived = await updateAgentMemory(project.id, { title: project.title, status: "archived" });
    await archiveAgentMemory(project.id);
    expect(global[0]).toMatchObject({ id: "memory-global", projectId: "", status: "active" });
    expect(projectRows[0]).toMatchObject({ id: "memory-project", projectId: "project-1" });
    expect(project).toMatchObject({ id: "memory-project", projectId: "project-1" });
    expect(archived.status).toBe("archived");
    expect(fetchMock.mock.calls.map(([url]) => String(url))).toEqual([
      expect.stringMatching(/\/memories$/),
      expect.stringMatching(/\/memories\?project_id=project-1$/),
      expect.stringMatching(/\/memories$/),
      expect.stringMatching(/\/memories\/memory-project$/),
      expect.stringMatching(/\/memories\/memory-project$/),
    ]);
    expect(fetchMock.mock.calls.map(([, init]) => init?.method)).toEqual([undefined, undefined, "POST", "PATCH", "DELETE"]);
    expect(JSON.parse(String(fetchMock.mock.calls[2][1]?.body))).toMatchObject({ project_id: "project-1" });
    expect(JSON.parse(String(fetchMock.mock.calls[3][1]?.body))).toEqual({ title: "Cohort design", lifecycle: "archived" });
  });

  it("creates and lists Project-scoped chat threads without changing global chat routes", async () => {
    const fetchMock = vi.fn()
      .mockResolvedValueOnce(jsonResponse({ data: { id: "thread-project", title: "YTHDF2", provider_id: "provider-1", project_id: "project-1" } }, 201))
      .mockResolvedValueOnce(jsonResponse({ data: [{ id: "thread-project", title: "YTHDF2", provider_id: "provider-1", project_id: "project-1" }] }));
    vi.stubGlobal("fetch", fetchMock);
    const created = await createProjectChatThread("project-1", { title: "YTHDF2", provider_id: "provider-1" });
    const listed = await listProjectChatThreads("project-1");
    expect(created).toMatchObject({ id: "thread-project", projectId: "project-1" });
    expect(listed[0]).toMatchObject({ id: "thread-project", projectId: "project-1" });
    expect(fetchMock.mock.calls.map(([url]) => String(url))).toEqual([
      expect.stringMatching(/\/threads$/),
      expect.stringMatching(/\/threads\?project_id=project-1$/),
    ]);
    expect(JSON.parse(String(fetchMock.mock.calls[0][1]?.body))).toMatchObject({ project_id: "project-1", scope: "project" });
  });

  it("routes search results to their global or Project-scoped chat", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(jsonResponse({
      chats: [
        { id: "thread-global", title: "Global YTHDF2" },
        { id: "thread-project", title: "Project YTHDF2", project_id: "project-1" },
      ],
    })));
    const results = await searchWorkspace("YTHDF2");
    expect(results).toEqual([
      expect.objectContaining({ id: "thread-global", href: "/chat/thread-global" }),
      expect.objectContaining({ id: "thread-project", href: "/projects/project-1/chat/thread-project" }),
    ]);
  });

  it("preserves the backend error reason when persisted failed tool events are reloaded", async () => {
    vi.stubGlobal("fetch", vi.fn()
      .mockResolvedValueOnce(jsonResponse({ data: { id: "chat-1", title: "Private Resource", provider_id: "provider-1" } }))
      .mockResolvedValueOnce(jsonResponse({ data: [{
        id: "message-3",
        role: "assistant",
        content: "I could not inspect that Resource.",
        created_at: "now",
        tool_events: [{ tool: "inspect_resource", status: "failed", error: "Resource is private for this provider" }],
        citations: [],
      }] })));
    const thread = await getChatThread("chat-1");
    expect(thread.messages[0].toolEvents[0]).toMatchObject({
      name: "inspect_resource",
      status: "failed",
      summary: "Resource is private for this provider",
    });
  });

  it("registers an ordinary user through the public account endpoint", async () => {
    const fetchMock = vi.fn().mockResolvedValue(jsonResponse({ data: { id: "user-1", role: "user" } }, 201));
    vi.stubGlobal("fetch", fetchMock);
    const result = await registerUser("Researcher", "researcher@example.org", "a-secure-password", "invite-v1-test");
    expect(result).toMatchObject({ authenticated: true, user_id: "user-1", role: "user" });
    expect(JSON.parse(String(fetchMock.mock.calls[0][1]?.body))).toEqual({ display_name: "Researcher", email: "researcher@example.org", password: "a-secure-password", invite_code: "invite-v1-test" });
  });

  it("omits the invitation field for open registration", async () => {
    const fetchMock = vi.fn().mockResolvedValue(jsonResponse({ data: { id: "user-2", role: "user" } }, 201));
    vi.stubGlobal("fetch", fetchMock);

    await expect(registerUser("Open User", "open@example.org", "a-secure-password")).resolves.toMatchObject({
      authenticated: true,
      user_id: "user-2",
      role: "user",
    });
    expect(JSON.parse(String(fetchMock.mock.calls[0][1]?.body))).toEqual({
      display_name: "Open User",
      email: "open@example.org",
      password: "a-secure-password",
    });
  });
});

function jsonResponse(value: unknown, status = 200) {
  return new Response(JSON.stringify(value), { status, headers: { "content-type": "application/json" } });
}
