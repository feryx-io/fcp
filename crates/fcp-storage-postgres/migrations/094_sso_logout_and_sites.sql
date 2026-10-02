-- Single sign-on, phase 3b (docs/IDENTITY_AND_DEVICES.md).
--
-- A session begun through an identity provider remembers which provider,
-- and the provider's own session id (`sid`) when its ID token carried one,
-- so the provider's back-channel logout can end it.
ALTER TABLE login_sessions ADD COLUMN IF NOT EXISTS idp_provider_id TEXT;
ALTER TABLE login_sessions ADD COLUMN IF NOT EXISTS idp_sid TEXT;
CREATE INDEX IF NOT EXISTS login_sessions_idp
    ON login_sessions (idp_provider_id, idp_sid)
    WHERE idp_provider_id IS NOT NULL;

-- The handoff carries the provider's session id to the session it becomes.
ALTER TABLE sso_handoffs ADD COLUMN IF NOT EXISTS idp_sid TEXT;

-- A site signs its people in through its tenant's providers itself
-- (docs/IDENTITY_AND_DEVICES.md Q11): providers and the identities at them
-- are in a site's slice (docs/HYBRID_CONTROL_PLANE.md §3.2). The journal
-- names a row by one column; an identity's is its provider and subject.
ALTER TABLE user_identities
    ADD COLUMN IF NOT EXISTS id TEXT GENERATED ALWAYS AS (provider_id || ':' || subject) STORED;

DROP TRIGGER IF EXISTS sync_journal ON identity_providers;
CREATE TRIGGER sync_journal AFTER INSERT OR UPDATE OR DELETE ON identity_providers
    FOR EACH ROW EXECUTE FUNCTION fcp_sync_journal('id', 'tenant_id');

DROP TRIGGER IF EXISTS sync_journal ON user_identities;
CREATE TRIGGER sync_journal AFTER INSERT OR UPDATE OR DELETE ON user_identities
    FOR EACH ROW EXECUTE FUNCTION fcp_sync_journal('id', 'tenant_id');
