-- Parking lots at a site (docs/PARKING.md phase 4): a lot is the cloud's
-- like the rest of the dial plan, journaled for the sites' links, which
-- write it into the site's own database. A site parks in its own Redis.
DROP TRIGGER IF EXISTS sync_journal ON parking_lots;
CREATE TRIGGER sync_journal AFTER INSERT OR UPDATE OR DELETE ON parking_lots
    FOR EACH ROW EXECUTE FUNCTION fcp_sync_journal('id', 'tenant_id');
