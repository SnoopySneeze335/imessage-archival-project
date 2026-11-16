-- user.db, schema version 1. See docs/SPEC.md, "Data model and storage", and
-- docs/decisions/0008-access-and-sign-in.md.
--
-- user.db holds what only the user can author. Unlike archive.db it cannot be
-- rebuilt from backups, so it is the file to back up. Never edit a released
-- migration; add a new one.
--
-- Times are UTC nanoseconds since 1970-01-01.

-- Settings by name (display timezone in Milestone 4d, and later more). Values
-- are JSON.
CREATE TABLE setting (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);

-- The one password, as an Argon2id PHC string. No row means first run.
CREATE TABLE password (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    hash TEXT NOT NULL,
    set_at INTEGER NOT NULL
);

-- Signed-in browsers. Only a SHA-256 of each session token is stored, so a
-- copy of user.db cannot be used to sign in.
CREATE TABLE session (
    token_hash TEXT PRIMARY KEY,
    created_at INTEGER NOT NULL,
    last_seen_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL
);
CREATE INDEX session_expires ON session (expires_at);
