CREATE TABLE run_approvals (
  id UUID PRIMARY KEY,
  run_id UUID NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
  resumed_run_id UUID UNIQUE REFERENCES runs(id) ON DELETE SET NULL,
  requested_by_user_id UUID NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  decided_by_user_id UUID REFERENCES users(id) ON DELETE RESTRICT,
  tool_call_id TEXT NOT NULL CHECK (length(tool_call_id) BETWEEN 1 AND 256),
  tool_name TEXT NOT NULL CHECK (length(tool_name) BETWEEN 1 AND 128),
  arguments_digest TEXT NOT NULL CHECK (arguments_digest ~ '^[0-9a-f]{64}$'),
  arguments JSONB NOT NULL CHECK (jsonb_typeof(arguments) = 'object'),
  risk TEXT NOT NULL CHECK (risk IN ('read', 'write', 'network', 'compute', 'destructive', 'admin')),
  approval_scope TEXT NOT NULL CHECK (approval_scope IN ('runtime.compute', 'project.write', 'runtime.cancel', 'artifact.register')),
  status TEXT NOT NULL DEFAULT 'pending'
    CHECK (status IN ('pending', 'approved', 'rejected', 'expired')),
  response_payload JSONB NOT NULL DEFAULT '{}'::jsonb CHECK (jsonb_typeof(response_payload) = 'object'),
  expires_at TIMESTAMPTZ NOT NULL,
  decided_at TIMESTAMPTZ,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  UNIQUE(run_id, tool_call_id)
);

CREATE INDEX run_approvals_pending_idx
  ON run_approvals(run_id, expires_at)
  WHERE status = 'pending';

CREATE INDEX run_approvals_resumed_idx
  ON run_approvals(resumed_run_id)
  WHERE resumed_run_id IS NOT NULL;
