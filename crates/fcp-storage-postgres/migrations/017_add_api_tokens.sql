-- API Tokens for programmatic access
CREATE TABLE IF NOT EXISTS api_tokens (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    token_hash  TEXT NOT NULL UNIQUE,
    prefix      TEXT NOT NULL,
    user_id     TEXT NOT NULL DEFAULT '',
    tenant_id   TEXT NOT NULL DEFAULT '_system',
    permissions JSONB NOT NULL DEFAULT '[]',
    expires_at  TIMESTAMPTZ,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_used_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_api_tokens_tenant ON api_tokens(tenant_id);
CREATE INDEX IF NOT EXISTS idx_api_tokens_hash ON api_tokens(token_hash);
