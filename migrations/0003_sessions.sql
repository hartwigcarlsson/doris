-- Login sessions. Operational state, not events: safe to purge.
-- No foreign key to users: that projection is emptied on rebuild.

CREATE TABLE sessions (
    token_hash TEXT PRIMARY KEY,
    user_id    TEXT NOT NULL,
    expires_at INTEGER NOT NULL -- unix seconds
);

CREATE INDEX sessions_expires_at ON sessions (expires_at);
