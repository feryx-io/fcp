-- Presence a person sets, synced with sites (docs/PRESENCE.md §3, as
-- contacts are in docs/RECORD_AND_CONTACTS.md §3).
--
-- Down, a status chosen or made in the cloud reaches a site through the
-- sync journal; up, one chosen or made at a site reaches the cloud
-- through the link's shipping (ship_seq). Each side takes the other's
-- copy only when it is newer (updated_at), so a change's echo is let go
-- and the later of two changes wins.
--
-- A delete has to cross both ways, and shipping carries rows, not
-- deletes:
-- - a chosen status cleared back to Available is a row saying
--   'available' with no note or time (the store reads it as no status);
-- - one of a user's own statuses deleted is a tombstone (deleted_at),
--   swept after 30 days like a contact's.
ALTER TABLE presence_statuses ADD COLUMN IF NOT EXISTS deleted_at TIMESTAMPTZ;
CREATE INDEX IF NOT EXISTS idx_presence_statuses_tombstones
    ON presence_statuses (deleted_at) WHERE deleted_at IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_presence_statuses_tenant ON presence_statuses (tenant_id);

ALTER TABLE user_presence ADD COLUMN IF NOT EXISTS ship_seq BIGINT;
ALTER TABLE user_presence ALTER COLUMN ship_seq SET DEFAULT nextval('fcp_ship_seq');
CREATE INDEX IF NOT EXISTS idx_user_presence_ship_seq
    ON user_presence (ship_seq) WHERE ship_seq IS NOT NULL;
DROP TRIGGER IF EXISTS user_presence_ship_seq ON user_presence;
CREATE TRIGGER user_presence_ship_seq BEFORE UPDATE ON user_presence
    FOR EACH ROW EXECUTE FUNCTION fcp_bump_ship_seq();

ALTER TABLE presence_statuses ADD COLUMN IF NOT EXISTS ship_seq BIGINT;
ALTER TABLE presence_statuses ALTER COLUMN ship_seq SET DEFAULT nextval('fcp_ship_seq');
CREATE INDEX IF NOT EXISTS idx_presence_statuses_ship_seq
    ON presence_statuses (ship_seq) WHERE ship_seq IS NOT NULL;
DROP TRIGGER IF EXISTS presence_statuses_ship_seq ON presence_statuses;
CREATE TRIGGER presence_statuses_ship_seq BEFORE UPDATE ON presence_statuses
    FOR EACH ROW EXECUTE FUNCTION fcp_bump_ship_seq();

DROP TRIGGER IF EXISTS sync_journal ON user_presence;
CREATE TRIGGER sync_journal AFTER INSERT OR UPDATE OR DELETE ON user_presence
    FOR EACH ROW EXECUTE FUNCTION fcp_sync_journal('user_id', 'tenant_id');

DROP TRIGGER IF EXISTS sync_journal ON presence_statuses;
CREATE TRIGGER sync_journal AFTER INSERT OR UPDATE OR DELETE ON presence_statuses
    FOR EACH ROW EXECUTE FUNCTION fcp_sync_journal('id', 'tenant_id');
