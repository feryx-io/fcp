-- Speech caps' warnings (docs/SPEECH_CAPS.md C2): each tenant's passing 80 %
-- and 100 % of a cap on a provider in a month, written once by the admin's
-- sweep so the warning is raised once.
CREATE TABLE IF NOT EXISTS speech_cap_notices (
    tenant_id TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    month DATE NOT NULL,
    level TEXT NOT NULL CHECK (level IN ('near', 'reached')),
    cap BIGINT NOT NULL,
    used BIGINT NOT NULL,
    noticed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, provider, month, level)
);

-- What the platform charges per million characters for each of its
-- providers: an estimated cost beside a tenant's usage, never a cap.
ALTER TABLE platform_speech ADD COLUMN IF NOT EXISTS prices JSONB NOT NULL DEFAULT '{}'::jsonb;
