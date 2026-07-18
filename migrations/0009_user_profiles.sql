ALTER TABLE users
  ADD COLUMN username TEXT,
  ADD COLUMN avatar_url TEXT;

UPDATE users
SET username = 'user-' || substring(replace(id::text, '-', ''), 1, 27)
WHERE username IS NULL;

ALTER TABLE users
  ALTER COLUMN username SET NOT NULL,
  ADD CONSTRAINT users_username_format_check
    CHECK (username ~ '^[a-z0-9][a-z0-9._-]{2,31}$'),
  ADD CONSTRAINT users_avatar_url_size_check
    CHECK (avatar_url IS NULL OR length(avatar_url) <= 700000);

CREATE UNIQUE INDEX users_username_unique_idx ON users(lower(username));
