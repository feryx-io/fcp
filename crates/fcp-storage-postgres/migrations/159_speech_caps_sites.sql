-- Speech caps at a site (docs/SPEECH_CAPS.md C3).
--
-- Down: the cloud's sweep keeps, for each tenant with a site, the cap that
-- applies to each capped provider and the month's synthesized characters
-- by where they were spoken ('' the cloud, else a site's id). It is
-- journaled, so it reaches the tenant's sites.
CREATE TABLE IF NOT EXISTS site_speech_caps (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    month DATE NOT NULL,
    cap BIGINT NOT NULL,
    used JSONB NOT NULL DEFAULT '{}'::jsonb,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS site_speech_caps_tenant ON site_speech_caps (tenant_id);
DROP TRIGGER IF EXISTS sync_journal ON site_speech_caps;
CREATE TRIGGER sync_journal AFTER INSERT OR UPDATE OR DELETE ON site_speech_caps
    FOR EACH ROW EXECUTE FUNCTION fcp_sync_journal('id', 'tenant_id');

-- At a site: what the link made of it — the cap, and what was spoken this
-- month everywhere but here (the site counts its own). Read with the
-- site's own caps and usage by its TTS server.
CREATE TABLE IF NOT EXISTS speech_cap_carried (
    tenant_id TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    month DATE NOT NULL,
    cap BIGINT NOT NULL,
    used_elsewhere BIGINT NOT NULL DEFAULT 0,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, provider)
);
