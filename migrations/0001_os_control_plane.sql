CREATE TABLE os_settings (
  singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
  registration_mode TEXT NOT NULL DEFAULT 'invite_only'
    CHECK (registration_mode IN ('disabled', 'invite_only', 'open')),
  instance_name TEXT NOT NULL DEFAULT 'Shennong OS',
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

INSERT INTO os_settings(singleton) VALUES (TRUE);

CREATE TABLE users (
  id UUID PRIMARY KEY,
  email TEXT NOT NULL,
  email_normalized TEXT NOT NULL UNIQUE,
  display_name TEXT NOT NULL CHECK (length(display_name) BETWEEN 1 AND 128),
  password_hash TEXT NOT NULL,
  role TEXT NOT NULL CHECK (role IN ('admin', 'user')),
  status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'disabled')),
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  CHECK (length(email) BETWEEN 3 AND 320),
  CHECK (email_normalized = lower(email_normalized))
);

CREATE TABLE sessions (
  id UUID PRIMARY KEY,
  user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  token_hash BYTEA NOT NULL UNIQUE,
  csrf_hash BYTEA NOT NULL,
  expires_at TIMESTAMPTZ NOT NULL,
  revoked_at TIMESTAMPTZ,
  ip_hash BYTEA,
  user_agent TEXT,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  last_seen_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  CHECK (user_agent IS NULL OR length(user_agent) <= 512)
);
CREATE INDEX sessions_user_active_idx ON sessions(user_id, expires_at DESC)
  WHERE revoked_at IS NULL;

CREATE TABLE registration_invites (
  id UUID PRIMARY KEY,
  code_hash BYTEA NOT NULL UNIQUE,
  code_prefix TEXT NOT NULL CHECK (length(code_prefix) BETWEEN 4 AND 24),
  created_by_user_id UUID NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  email_constraint TEXT,
  max_uses INTEGER NOT NULL DEFAULT 1 CHECK (max_uses BETWEEN 1 AND 10000),
  use_count INTEGER NOT NULL DEFAULT 0 CHECK (use_count >= 0 AND use_count <= max_uses),
  expires_at TIMESTAMPTZ NOT NULL,
  revoked_at TIMESTAMPTZ,
  note TEXT NOT NULL DEFAULT '' CHECK (length(note) <= 1024),
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  CHECK (email_constraint IS NULL OR email_constraint = lower(email_constraint))
);
CREATE INDEX registration_invites_active_idx
  ON registration_invites(expires_at, use_count)
  WHERE revoked_at IS NULL;
CREATE INDEX registration_invites_creator_idx
  ON registration_invites(created_by_user_id, created_at DESC);

CREATE TABLE registration_invite_redemptions (
  id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  invite_id UUID NOT NULL REFERENCES registration_invites(id) ON DELETE RESTRICT,
  user_id UUID NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  normalized_email TEXT NOT NULL,
  ip_hash BYTEA,
  redeemed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  UNIQUE(invite_id, user_id)
);
CREATE INDEX registration_invite_redemptions_user_idx
  ON registration_invite_redemptions(user_id, redeemed_at DESC);

CREATE TABLE projects (
  id UUID PRIMARY KEY,
  owner_user_id UUID NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 200),
  description TEXT NOT NULL DEFAULT '' CHECK (length(description) <= 4096),
  visibility TEXT NOT NULL DEFAULT 'private' CHECK (visibility IN ('private', 'public')),
  status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'archived')),
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX projects_owner_status_idx ON projects(owner_user_id, status, updated_at DESC);

CREATE TABLE project_members (
  project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  role TEXT NOT NULL CHECK (role IN ('owner', 'admin', 'editor', 'viewer')),
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  PRIMARY KEY(project_id, user_id)
);
CREATE INDEX project_members_user_idx ON project_members(user_id, project_id);

CREATE TABLE model_providers (
  id UUID PRIMARY KEY,
  owner_user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 128),
  provider_kind TEXT NOT NULL CHECK (provider_kind IN ('openai', 'deepseek', 'ollama', 'openai-compatible')),
  base_url TEXT NOT NULL CHECK (length(base_url) BETWEEN 8 AND 2048),
  model TEXT NOT NULL CHECK (length(model) BETWEEN 1 AND 256),
  data_policy TEXT NOT NULL DEFAULT 'public_only' CHECK (data_policy IN ('public_only', 'allow_private')),
  encrypted_api_key BYTEA,
  key_version INTEGER NOT NULL DEFAULT 1 CHECK (key_version > 0),
  enabled BOOLEAN NOT NULL DEFAULT TRUE,
  is_default BOOLEAN NOT NULL DEFAULT FALSE,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  UNIQUE(owner_user_id, name)
);
CREATE UNIQUE INDEX model_providers_owner_default_idx
  ON model_providers(owner_user_id) WHERE is_default;
CREATE INDEX model_providers_owner_enabled_idx
  ON model_providers(owner_user_id, enabled, updated_at DESC);

CREATE TABLE threads (
  id UUID PRIMARY KEY,
  project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  owner_user_id UUID NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  provider_id UUID REFERENCES model_providers(id) ON DELETE SET NULL,
  title TEXT NOT NULL DEFAULT 'New chat' CHECK (length(title) BETWEEN 1 AND 200),
  status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'archived')),
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX threads_project_updated_idx ON threads(project_id, updated_at DESC);
CREATE INDEX threads_owner_updated_idx ON threads(owner_user_id, updated_at DESC);

CREATE TABLE messages (
  id UUID PRIMARY KEY,
  thread_id UUID NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
  role TEXT NOT NULL CHECK (role IN ('user', 'assistant', 'tool', 'system')),
  content_json JSONB NOT NULL,
  status TEXT NOT NULL DEFAULT 'completed' CHECK (status IN ('pending', 'completed', 'failed', 'cancelled')),
  attachments JSONB NOT NULL DEFAULT '[]'::jsonb CHECK (jsonb_typeof(attachments) = 'array'),
  metadata JSONB NOT NULL DEFAULT '{}'::jsonb CHECK (jsonb_typeof(metadata) = 'object'),
  idempotency_key TEXT CHECK (idempotency_key IS NULL OR length(idempotency_key) BETWEEN 8 AND 200),
  CHECK (octet_length(content_json::text) <= 1048576),
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX messages_thread_created_idx ON messages(thread_id, created_at, id);
CREATE UNIQUE INDEX messages_thread_idempotency_idx
  ON messages(thread_id, idempotency_key) WHERE idempotency_key IS NOT NULL;

CREATE TABLE runs (
  id UUID PRIMARY KEY,
  project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  thread_id UUID NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
  parent_run_id UUID REFERENCES runs(id) ON DELETE SET NULL,
  requested_by_user_id UUID NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  status TEXT NOT NULL DEFAULT 'queued'
    CHECK (status IN ('queued', 'running', 'waiting_approval', 'succeeded', 'failed', 'failed_validation', 'cancelled')),
  input JSONB NOT NULL DEFAULT '{}'::jsonb CHECK (jsonb_typeof(input) = 'object'),
  output JSONB NOT NULL DEFAULT '{}'::jsonb CHECK (jsonb_typeof(output) = 'object'),
  error JSONB NOT NULL DEFAULT '{}'::jsonb CHECK (jsonb_typeof(error) = 'object'),
  metadata JSONB NOT NULL DEFAULT '{}'::jsonb CHECK (jsonb_typeof(metadata) = 'object'),
  capability_token_hash BYTEA,
  capability_expires_at TIMESTAMPTZ,
  started_at TIMESTAMPTZ,
  finished_at TIMESTAMPTZ,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX runs_project_status_idx ON runs(project_id, status, created_at DESC);
CREATE INDEX runs_thread_created_idx ON runs(thread_id, created_at DESC);
CREATE INDEX runs_parent_idx ON runs(parent_run_id) WHERE parent_run_id IS NOT NULL;
CREATE INDEX runs_capability_expiry_idx ON runs(capability_expires_at)
  WHERE capability_token_hash IS NOT NULL;

CREATE TABLE run_events (
  id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  run_id UUID NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
  event_type TEXT NOT NULL CHECK (length(event_type) BETWEEN 1 AND 128),
  payload JSONB NOT NULL DEFAULT '{}'::jsonb CHECK (jsonb_typeof(payload) = 'object'),
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX run_events_run_id_idx ON run_events(run_id, id);

CREATE TABLE run_tool_grants (
  id UUID PRIMARY KEY,
  run_id UUID NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
  tool_call_id TEXT NOT NULL CHECK (length(tool_call_id) BETWEEN 1 AND 256),
  tool_name TEXT NOT NULL CHECK (length(tool_name) BETWEEN 1 AND 128),
  arguments_digest TEXT NOT NULL CHECK (arguments_digest ~ '^[0-9a-f]{64}$'),
  risk TEXT NOT NULL CHECK (risk IN ('read', 'write', 'network', 'compute', 'destructive', 'admin')),
  execution_token_hash BYTEA NOT NULL,
  decision TEXT NOT NULL CHECK (decision IN ('allowed', 'denied')),
  denial_reason TEXT,
  expires_at TIMESTAMPTZ NOT NULL,
  used_at TIMESTAMPTZ,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  UNIQUE(run_id, tool_call_id)
);
CREATE INDEX run_tool_grants_active_idx ON run_tool_grants(run_id, expires_at)
  WHERE decision = 'allowed' AND used_at IS NULL;

CREATE TABLE task_plans (
  run_id UUID PRIMARY KEY REFERENCES runs(id) ON DELETE CASCADE,
  version INTEGER NOT NULL DEFAULT 1 CHECK (version > 0),
  items JSONB NOT NULL DEFAULT '[]'::jsonb CHECK (jsonb_typeof(items) = 'array'),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE jobs (
  id UUID PRIMARY KEY,
  project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  run_id UUID REFERENCES runs(id) ON DELETE SET NULL,
  kind TEXT NOT NULL CHECK (length(kind) BETWEEN 1 AND 128),
  status TEXT NOT NULL DEFAULT 'queued'
    CHECK (status IN ('queued', 'preparing', 'running', 'cancel_requested', 'succeeded', 'failed', 'cancelled', 'timed_out', 'lost')),
  spec JSONB NOT NULL DEFAULT '{}'::jsonb CHECK (jsonb_typeof(spec) = 'object'),
  result JSONB NOT NULL DEFAULT '{}'::jsonb CHECK (jsonb_typeof(result) = 'object'),
  created_by_user_id UUID NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX jobs_project_status_idx ON jobs(project_id, status, created_at DESC);
CREATE INDEX jobs_run_idx ON jobs(run_id) WHERE run_id IS NOT NULL;

CREATE TABLE runtime_sessions (
  id UUID PRIMARY KEY,
  project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  created_by_user_id UUID NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  kind TEXT NOT NULL CHECK (kind IN ('rstudio', 'jupyterlab')),
  worker_profile TEXT NOT NULL CHECK (length(worker_profile) BETWEEN 1 AND 128),
  status TEXT NOT NULL DEFAULT 'starting'
    CHECK (status IN ('starting', 'running', 'stop_requested', 'stopped', 'failed', 'expired', 'lost')),
  runtime_view JSONB NOT NULL DEFAULT '{}'::jsonb CHECK (jsonb_typeof(runtime_view) = 'object'),
  expires_at TIMESTAMPTZ,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX runtime_sessions_project_status_idx
  ON runtime_sessions(project_id, status, created_at DESC);

CREATE TABLE artifacts (
  id UUID PRIMARY KEY,
  project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  job_id UUID REFERENCES jobs(id) ON DELETE SET NULL,
  kind TEXT NOT NULL CHECK (length(kind) BETWEEN 1 AND 128),
  name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 256),
  locator TEXT NOT NULL CHECK (length(locator) BETWEEN 1 AND 4096),
  media_type TEXT,
  size_bytes BIGINT CHECK (size_bytes IS NULL OR size_bytes >= 0),
  content_sha256 TEXT CHECK (content_sha256 IS NULL OR content_sha256 ~ '^[0-9a-f]{64}$'),
  metadata JSONB NOT NULL DEFAULT '{}'::jsonb CHECK (jsonb_typeof(metadata) = 'object'),
  created_by_user_id UUID NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX artifacts_project_created_idx ON artifacts(project_id, created_at DESC);
CREATE INDEX artifacts_job_idx ON artifacts(job_id) WHERE job_id IS NOT NULL;

CREATE TABLE memories (
  id UUID PRIMARY KEY,
  owner_user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  project_id UUID REFERENCES projects(id) ON DELETE CASCADE,
  title TEXT NOT NULL CHECK (length(title) BETWEEN 1 AND 128),
  source_kind TEXT NOT NULL DEFAULT 'manual' CHECK (source_kind IN ('manual', 'conversation', 'imported')),
  source_id TEXT CHECK (source_id IS NULL OR length(source_id) <= 512),
  lifecycle TEXT NOT NULL DEFAULT 'active' CHECK (lifecycle IN ('active', 'archived')),
  current_version INTEGER NOT NULL DEFAULT 0 CHECK (current_version >= 0),
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX memories_owner_global_idx ON memories(owner_user_id, updated_at DESC)
  WHERE project_id IS NULL;
CREATE INDEX memories_project_owner_idx ON memories(project_id, owner_user_id, updated_at DESC)
  WHERE project_id IS NOT NULL;

CREATE TABLE memory_versions (
  memory_id UUID NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
  version INTEGER NOT NULL CHECK (version > 0),
  content TEXT NOT NULL CHECK (octet_length(content) BETWEEN 1 AND 65536),
  content_sha256 TEXT NOT NULL CHECK (content_sha256 ~ '^[0-9a-f]{64}$'),
  change_note TEXT NOT NULL DEFAULT '' CHECK (length(change_note) <= 1024),
  created_by_user_id UUID NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  PRIMARY KEY(memory_id, version)
);

CREATE TABLE skills (
  id UUID PRIMARY KEY,
  owner_user_id UUID REFERENCES users(id) ON DELETE CASCADE,
  slug TEXT NOT NULL CHECK (slug ~ '^[a-z0-9][a-z0-9-]{0,63}$'),
  name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 128),
  description TEXT NOT NULL DEFAULT '' CHECK (length(description) <= 1024),
  trust_level TEXT NOT NULL
    CHECK (trust_level IN ('builtin_signed', 'admin_curated', 'user', 'generated')),
  lifecycle TEXT NOT NULL DEFAULT 'draft'
    CHECK (lifecycle IN ('draft', 'active', 'disabled', 'archived')),
  manifest JSONB NOT NULL DEFAULT '{}'::jsonb CHECK (jsonb_typeof(manifest) = 'object'),
  current_version INTEGER NOT NULL DEFAULT 0 CHECK (current_version >= 0),
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  CHECK (
    (trust_level IN ('builtin_signed', 'admin_curated') AND owner_user_id IS NULL)
    OR (trust_level IN ('user', 'generated') AND owner_user_id IS NOT NULL)
  )
);
CREATE UNIQUE INDEX skills_global_slug_idx ON skills(slug) WHERE owner_user_id IS NULL;
CREATE UNIQUE INDEX skills_owner_slug_idx ON skills(owner_user_id, slug) WHERE owner_user_id IS NOT NULL;
CREATE INDEX skills_owner_lifecycle_idx ON skills(owner_user_id, lifecycle, updated_at DESC);

CREATE TABLE skill_versions (
  skill_id UUID NOT NULL REFERENCES skills(id) ON DELETE CASCADE,
  version INTEGER NOT NULL CHECK (version > 0),
  content TEXT NOT NULL CHECK (octet_length(content) BETWEEN 1 AND 65536),
  content_sha256 TEXT NOT NULL CHECK (content_sha256 ~ '^[0-9a-f]{64}$'),
  package_version TEXT NOT NULL DEFAULT '1',
  change_note TEXT NOT NULL DEFAULT '' CHECK (length(change_note) <= 1024),
  created_by_user_id UUID REFERENCES users(id) ON DELETE RESTRICT,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  PRIMARY KEY(skill_id, version)
);

CREATE TABLE thread_skills (
  thread_id UUID NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
  skill_id UUID NOT NULL REFERENCES skills(id) ON DELETE CASCADE,
  skill_version INTEGER NOT NULL CHECK (skill_version > 0),
  enabled BOOLEAN NOT NULL DEFAULT TRUE,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  PRIMARY KEY(thread_id, skill_id),
  FOREIGN KEY(skill_id, skill_version) REFERENCES skill_versions(skill_id, version) ON DELETE RESTRICT
);
CREATE INDEX thread_skills_skill_idx ON thread_skills(skill_id, skill_version);

CREATE TABLE audit_events (
  id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  actor_user_id UUID REFERENCES users(id) ON DELETE SET NULL,
  project_id UUID REFERENCES projects(id) ON DELETE SET NULL,
  action TEXT NOT NULL CHECK (length(action) BETWEEN 1 AND 128),
  target_type TEXT NOT NULL CHECK (length(target_type) BETWEEN 1 AND 128),
  target_id TEXT,
  request_id UUID,
  ip_hash BYTEA,
  details JSONB NOT NULL DEFAULT '{}'::jsonb CHECK (jsonb_typeof(details) = 'object'),
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX audit_events_actor_created_idx ON audit_events(actor_user_id, created_at DESC);
CREATE INDEX audit_events_project_created_idx ON audit_events(project_id, created_at DESC)
  WHERE project_id IS NOT NULL;

CREATE OR REPLACE FUNCTION immutable_version_row()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
  RAISE EXCEPTION 'version rows are immutable';
END;
$$;

CREATE TRIGGER memory_versions_immutable
BEFORE UPDATE ON memory_versions
FOR EACH ROW EXECUTE FUNCTION immutable_version_row();

CREATE TRIGGER skill_versions_immutable
BEFORE UPDATE ON skill_versions
FOR EACH ROW EXECUTE FUNCTION immutable_version_row();

CREATE OR REPLACE FUNCTION run_project_scope_guard()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
  IF NOT EXISTS (
    SELECT 1 FROM threads t
    WHERE t.id = NEW.thread_id AND t.project_id = NEW.project_id
  ) THEN
    RAISE EXCEPTION 'run and thread project scopes differ';
  END IF;
  IF NEW.parent_run_id IS NOT NULL AND NOT EXISTS (
    SELECT 1 FROM runs r
    WHERE r.id = NEW.parent_run_id AND r.project_id = NEW.project_id
      AND r.thread_id = NEW.thread_id
  ) THEN
    RAISE EXCEPTION 'parent and child run scopes differ';
  END IF;
  RETURN NEW;
END;
$$;

CREATE TRIGGER runs_project_scope_guard
BEFORE INSERT OR UPDATE OF project_id, thread_id, parent_run_id ON runs
FOR EACH ROW EXECUTE FUNCTION run_project_scope_guard();

CREATE OR REPLACE FUNCTION child_project_scope_guard()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
  IF NEW.run_id IS NOT NULL AND NOT EXISTS (
    SELECT 1 FROM runs r WHERE r.id = NEW.run_id AND r.project_id = NEW.project_id
  ) THEN
    RAISE EXCEPTION 'child and run project scopes differ';
  END IF;
  RETURN NEW;
END;
$$;

CREATE TRIGGER jobs_project_scope_guard
BEFORE INSERT OR UPDATE OF project_id, run_id ON jobs
FOR EACH ROW EXECUTE FUNCTION child_project_scope_guard();

CREATE OR REPLACE FUNCTION artifact_project_scope_guard()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
  IF NEW.job_id IS NOT NULL AND NOT EXISTS (
    SELECT 1 FROM jobs j WHERE j.id = NEW.job_id AND j.project_id = NEW.project_id
  ) THEN
    RAISE EXCEPTION 'artifact and job project scopes differ';
  END IF;
  RETURN NEW;
END;
$$;

CREATE TRIGGER artifacts_project_scope_guard
BEFORE INSERT OR UPDATE OF project_id, job_id ON artifacts
FOR EACH ROW EXECUTE FUNCTION artifact_project_scope_guard();
