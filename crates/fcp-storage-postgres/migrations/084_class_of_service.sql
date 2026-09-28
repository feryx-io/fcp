-- Enterprise calling, phase 7 (docs/ENTERPRISE_CALLING.md §3.7): a
-- tenant's classes of service — what its users may dial, how many calls
-- they may be on, how their caller ID leaves — and the tenant's calling
-- defaults: its country, its timezone, its own word on number ranges.
-- A class is its JSON, keyed by its id.
CREATE TABLE IF NOT EXISTS classes_of_service (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    name TEXT NOT NULL,
    class_json JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_classes_of_service_tenant ON classes_of_service (tenant_id);

-- A site's slice includes its tenant's classes (docs/HYBRID_CONTROL_PLANE.md §3.2).
DROP TRIGGER IF EXISTS sync_journal ON classes_of_service;
CREATE TRIGGER sync_journal AFTER INSERT OR UPDATE OR DELETE ON classes_of_service
    FOR EACH ROW EXECUTE FUNCTION fcp_sync_journal('id', 'tenant_id');

-- `{ country, timezone, destination_overrides }`; empty is North American
-- numbering and UTC, as before.
ALTER TABLE tenants ADD COLUMN IF NOT EXISTS calling_json JSONB NOT NULL DEFAULT '{}'::jsonb;
