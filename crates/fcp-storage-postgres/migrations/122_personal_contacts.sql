-- A user's own contacts (docs/RECORD_AND_CONTACTS.md §3): kept by FCP so
-- every app the user signs in to has them, and synced with the tenant's
-- sites both ways.
--
-- Down, a contact written in the cloud reaches a site through the sync
-- journal like the tenant's configuration; up, one written at a site
-- reaches the cloud through the link's shipping (ship_seq), like its
-- records. Each side takes the other's copy only when it is newer
-- (updated_at), so a change's echo is let go and the later of two edits
-- wins. A delete is a tombstone (deleted_at), swept after 30 days.
CREATE TABLE IF NOT EXISTS personal_contacts (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    owner_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    company TEXT,
    numbers JSONB NOT NULL DEFAULT '[]'::jsonb,
    email TEXT,
    notes TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    deleted_at TIMESTAMPTZ,
    ship_seq BIGINT DEFAULT nextval('fcp_ship_seq')
);
CREATE INDEX IF NOT EXISTS idx_personal_contacts_owner
    ON personal_contacts (owner_user_id, lower(name), id) WHERE deleted_at IS NULL;
CREATE INDEX IF NOT EXISTS idx_personal_contacts_tenant ON personal_contacts (tenant_id);
CREATE INDEX IF NOT EXISTS idx_personal_contacts_ship_seq
    ON personal_contacts (ship_seq) WHERE ship_seq IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_personal_contacts_tombstones
    ON personal_contacts (deleted_at) WHERE deleted_at IS NOT NULL;

DROP TRIGGER IF EXISTS personal_contacts_ship_seq ON personal_contacts;
CREATE TRIGGER personal_contacts_ship_seq BEFORE UPDATE ON personal_contacts
    FOR EACH ROW EXECUTE FUNCTION fcp_bump_ship_seq();

DROP TRIGGER IF EXISTS sync_journal ON personal_contacts;
CREATE TRIGGER sync_journal AFTER INSERT OR UPDATE OR DELETE ON personal_contacts
    FOR EACH ROW EXECUTE FUNCTION fcp_sync_journal('id', 'tenant_id');
