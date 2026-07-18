CREATE TABLE ide_launch_tickets (
  id UUID PRIMARY KEY,
  runtime_session_id UUID NOT NULL REFERENCES runtime_sessions(id) ON DELETE CASCADE,
  user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  issued_from_session_id UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  token_hash BYTEA NOT NULL UNIQUE CHECK (octet_length(token_hash) = 32),
  expires_at TIMESTAMPTZ NOT NULL,
  used_at TIMESTAMPTZ,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  CHECK (expires_at > created_at)
);
CREATE INDEX ide_launch_tickets_active_idx
  ON ide_launch_tickets(expires_at)
  WHERE used_at IS NULL;

CREATE TABLE ide_access_sessions (
  id UUID PRIMARY KEY,
  runtime_session_id UUID NOT NULL REFERENCES runtime_sessions(id) ON DELETE CASCADE,
  user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  token_hash BYTEA NOT NULL UNIQUE CHECK (octet_length(token_hash) = 32),
  expires_at TIMESTAMPTZ NOT NULL,
  revoked_at TIMESTAMPTZ,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  last_seen_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  CHECK (expires_at > created_at)
);
CREATE INDEX ide_access_sessions_active_idx
  ON ide_access_sessions(runtime_session_id, expires_at)
  WHERE revoked_at IS NULL;

CREATE OR REPLACE FUNCTION ide_access_owner_guard()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
  IF NOT EXISTS (
    SELECT 1 FROM runtime_sessions s
    WHERE s.id = NEW.runtime_session_id
      AND s.created_by_user_id = NEW.user_id
  ) THEN
    RAISE EXCEPTION 'IDE access and Runtime Session owners differ';
  END IF;
  RETURN NEW;
END;
$$;

CREATE TRIGGER ide_launch_tickets_owner_guard
BEFORE INSERT OR UPDATE OF runtime_session_id, user_id ON ide_launch_tickets
FOR EACH ROW EXECUTE FUNCTION ide_access_owner_guard();

CREATE TRIGGER ide_access_sessions_owner_guard
BEFORE INSERT OR UPDATE OF runtime_session_id, user_id ON ide_access_sessions
FOR EACH ROW EXECUTE FUNCTION ide_access_owner_guard();
