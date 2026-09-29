-- The developer surface, phase 1 (docs/DEVELOPER_SURFACE.md §3): an API
-- token has a scope (the admin API, or only its maker's own /me routes),
-- is revoked by marking rather than deleting, and can be rotated — the
-- replaced secret's hash kept and honoured until its grace period ends.
-- `user_id` (already present, always empty until now) names the maker.
ALTER TABLE api_tokens ADD COLUMN IF NOT EXISTS scope TEXT NOT NULL DEFAULT 'admin';
ALTER TABLE api_tokens ADD COLUMN IF NOT EXISTS revoked_at TIMESTAMPTZ;
ALTER TABLE api_tokens ADD COLUMN IF NOT EXISTS revoked_by TEXT;
ALTER TABLE api_tokens ADD COLUMN IF NOT EXISTS previous_token_hash TEXT;
ALTER TABLE api_tokens ADD COLUMN IF NOT EXISTS previous_expires_at TIMESTAMPTZ;
ALTER TABLE api_tokens ADD COLUMN IF NOT EXISTS rotated_at TIMESTAMPTZ;

CREATE INDEX IF NOT EXISTS idx_api_tokens_previous_hash
    ON api_tokens(previous_token_hash) WHERE previous_token_hash IS NOT NULL;
