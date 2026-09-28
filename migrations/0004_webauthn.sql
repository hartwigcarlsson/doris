-- WebAuthn ceremony state (single use, short-lived) and server secrets.
-- webauthn_ceremonies is operational state, not events: safe to purge.
-- server_secrets must NOT be purged: it keeps the fake-credential key that
-- makes fake credential ids for unknown emails stable across restarts.
-- Changing it would reveal which emails are registered.

CREATE TABLE webauthn_ceremonies (
    ceremony_id TEXT PRIMARY KEY,
    data        TEXT NOT NULL CHECK (json_valid(data)),
    expires_at  INTEGER NOT NULL -- unix seconds
);

CREATE TABLE server_secrets (
    name  TEXT PRIMARY KEY,
    value BLOB NOT NULL
);
