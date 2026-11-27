-- Speech at a site (docs/TENANT_SPEECH.md S5).
--
-- Down: a tenant's speech settings and its own accounts are in a site's
-- slice (docs/HYBRID_CONTROL_PLANE.md §3.2), an account's key crossing the
-- link opened and sealed again with the site's own key, like an identity
-- provider's secret.
DROP TRIGGER IF EXISTS sync_journal ON tenant_speech;
CREATE TRIGGER sync_journal AFTER INSERT OR UPDATE OR DELETE ON tenant_speech
    FOR EACH ROW EXECUTE FUNCTION fcp_sync_journal('tenant_id', 'tenant_id');

DROP TRIGGER IF EXISTS sync_journal ON speech_accounts;
CREATE TRIGGER sync_journal AFTER INSERT OR UPDATE OR DELETE ON speech_accounts
    FOR EACH ROW EXECUTE FUNCTION fcp_sync_journal('id', 'tenant_id');

-- Up: a site's usage comes up with its records. The cloud keeps each
-- site's hours apart from its own (`site_id`, '' for its own), set to what
-- the site counted rather than added to, so a row shipped twice counts
-- once. `ship_seq` orders a site's own rows for the link.
ALTER TABLE speech_usage ADD COLUMN IF NOT EXISTS site_id TEXT NOT NULL DEFAULT '';
ALTER TABLE speech_usage DROP CONSTRAINT IF EXISTS speech_usage_pkey;
ALTER TABLE speech_usage ADD PRIMARY KEY (tenant_id, hour, provider, site_id);
ALTER TABLE speech_usage ADD COLUMN IF NOT EXISTS ship_seq BIGINT;
ALTER TABLE speech_usage ALTER COLUMN ship_seq SET DEFAULT nextval('fcp_ship_seq');
CREATE INDEX IF NOT EXISTS idx_speech_usage_ship_seq
    ON speech_usage (ship_seq) WHERE ship_seq IS NOT NULL;
DROP TRIGGER IF EXISTS speech_usage_ship_seq ON speech_usage;
CREATE TRIGGER speech_usage_ship_seq BEFORE UPDATE ON speech_usage
    FOR EACH ROW EXECUTE FUNCTION fcp_bump_ship_seq();
