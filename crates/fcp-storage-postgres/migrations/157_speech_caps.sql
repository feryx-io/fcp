-- Speech spending caps (docs/SPEECH_CAPS.md C1): synthesized characters a
-- tenant may speak with one provider in a calendar month (UTC). The
-- platform caps its providers, by default (no tenant) and per tenant
-- (where a null limit means none for that tenant); a tenant caps its own
-- accounts and, lower only, the platform's providers.
CREATE TABLE IF NOT EXISTS speech_caps (
    tenant_id TEXT REFERENCES tenants(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('platform', 'tenant')),
    monthly_characters BIGINT CHECK (monthly_characters IS NULL OR monthly_characters >= 0),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    -- A tenant's own cap is on a tenant and has a limit; the platform's
    -- default has a limit.
    CHECK (kind = 'platform' OR (tenant_id IS NOT NULL AND monthly_characters IS NOT NULL)),
    CHECK (tenant_id IS NOT NULL OR monthly_characters IS NOT NULL)
);

CREATE UNIQUE INDEX IF NOT EXISTS speech_caps_one
    ON speech_caps ((COALESCE(tenant_id, '')), provider, kind);
