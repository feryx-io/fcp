-- A tenant's firmware targets, synced to its sites (docs/PROVISIONING.md
-- §11, docs/IDENTITY_AND_DEVICES.md phase 9c): each change journaled, the
-- target carrying its image to the site.
DROP TRIGGER IF EXISTS sync_journal ON firmware_targets;
CREATE TRIGGER sync_journal AFTER INSERT OR UPDATE OR DELETE ON firmware_targets
    FOR EACH ROW EXECUTE FUNCTION fcp_sync_journal('id', 'tenant_id');
