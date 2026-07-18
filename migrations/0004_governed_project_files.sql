-- OS-owned virtual project files back the V1 project.* governed tools.
-- Contents are untrusted text records: no host path, mount, or executable bit is stored.
CREATE TABLE project_files (
  project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  path TEXT NOT NULL CHECK (
    path LIKE 'project://current/%'
    AND length(path) BETWEEN 19 AND 1024
    AND path !~ '[[:cntrl:]]'
    AND strpos(path, E'\\') = 0
    AND path !~ '(^|/)\.{1,2}(/|$)'
  ),
  content TEXT NOT NULL CHECK (octet_length(content) <= 1048576),
  content_sha256 TEXT NOT NULL CHECK (content_sha256 ~ '^[0-9a-f]{64}$'),
  version INTEGER NOT NULL DEFAULT 1 CHECK (version > 0),
  created_by_user_id UUID NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  updated_by_user_id UUID NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  PRIMARY KEY(project_id, path)
);

CREATE INDEX project_files_project_updated_idx
  ON project_files(project_id, updated_at DESC, path);
