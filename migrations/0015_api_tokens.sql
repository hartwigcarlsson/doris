-- Projection of ApiTokenCreated/ApiTokenRevoked (streams api-token-{id}).
-- Rebuildable. Only the SHA-256 of a token is stored.
CREATE TABLE api_tokens (
    token_id    TEXT PRIMARY KEY,
    user_id     TEXT NOT NULL,
    name        TEXT NOT NULL,
    token_hash  TEXT NOT NULL UNIQUE,
    grants      TEXT NOT NULL,      -- JSON, as in the event
    created_at  TEXT NOT NULL,      -- recorded_at
    expires_at  INTEGER NOT NULL,   -- unix seconds
    revoked_at  TEXT
);

CREATE INDEX api_tokens_user ON api_tokens (user_id);

-- When a token was last used. Operational state, not events: safe to purge.
CREATE TABLE api_token_usage (
    token_id     TEXT PRIMARY KEY,
    last_used_at INTEGER NOT NULL   -- unix seconds
);
