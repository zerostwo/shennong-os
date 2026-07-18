ALTER TABLE threads ALTER COLUMN project_id DROP NOT NULL;
ALTER TABLE runs ALTER COLUMN project_id DROP NOT NULL;

CREATE OR REPLACE FUNCTION run_project_scope_guard()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
  IF NOT EXISTS (
    SELECT 1 FROM threads t
    WHERE t.id = NEW.thread_id
      AND t.project_id IS NOT DISTINCT FROM NEW.project_id
  ) THEN
    RAISE EXCEPTION 'run and thread project scopes differ';
  END IF;
  IF NEW.parent_run_id IS NOT NULL AND NOT EXISTS (
    SELECT 1 FROM runs r
    WHERE r.id = NEW.parent_run_id
      AND r.project_id IS NOT DISTINCT FROM NEW.project_id
      AND r.thread_id = NEW.thread_id
  ) THEN
    RAISE EXCEPTION 'parent and child run scopes differ';
  END IF;
  RETURN NEW;
END;
$$;
