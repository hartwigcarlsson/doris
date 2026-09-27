-- Projections of the user-* and invitation-* streams. Rebuildable from events.

CREATE TABLE users (
    user_id       TEXT PRIMARY KEY,
    email         TEXT NOT NULL UNIQUE,
    display_name  TEXT NOT NULL,
    role          TEXT NOT NULL,
    registered_at TEXT NOT NULL
);

CREATE TABLE passkeys (
    credential_id TEXT PRIMARY KEY,
    user_id       TEXT NOT NULL REFERENCES users (user_id),
    name          TEXT NOT NULL,
    passkey       TEXT NOT NULL,
    added_at      TEXT NOT NULL,
    last_used_at  TEXT
);

CREATE TABLE invitations (
    invitation_id TEXT PRIMARY KEY,
    email         TEXT NOT NULL,
    token_hash    TEXT NOT NULL UNIQUE,
    created_by    TEXT NOT NULL,
    expires_at    INTEGER NOT NULL, -- unix seconds, for range queries
    accepted_by   TEXT
);

CREATE INDEX invitations_email ON invitations (email);
