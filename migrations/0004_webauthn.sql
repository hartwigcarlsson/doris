-- WebAuthn ceremony state (single use, short-lived) and server secrets.
-- Operational state, not events: safe to purge.

CREATE TABLE webauthn_ceremonies (
    ceremony_id TEXT PRIMARY KEY,
    data        TEXT NOT NULL CHECK (json_valid(data)),
    expires_at  INTEGER NOT NULL -- unix seconds
);

CREATE TABLE server_secrets (
    name  TEXT PRIMARY KEY,
    value BLOB NOT NULL
);
