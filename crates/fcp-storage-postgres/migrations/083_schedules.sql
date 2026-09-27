-- Enterprise calling, phase 5 (docs/ENTERPRISE_CALLING.md §3.5): a
-- tenant's schedules — business hours and holidays in a timezone — and
-- where a number's calls go when its schedule is closed or on a holiday.
-- A schedule is its JSON, keyed by its id.
CREATE TABLE IF NOT EXISTS schedules (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    name TEXT NOT NULL,
    schedule_json JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_schedules_tenant ON schedules (tenant_id);

-- A site's slice includes its tenant's schedules (docs/HYBRID_CONTROL_PLANE.md §3.2).
DROP TRIGGER IF EXISTS sync_journal ON schedules;
CREATE TRIGGER sync_journal AFTER INSERT OR UPDATE OR DELETE ON schedules
    FOR EACH ROW EXECUTE FUNCTION fcp_sync_journal('id', 'tenant_id');

-- A number's schedule, and its targets when closed and on a holiday
-- (`voicemail`, `number:+…`, `ext:…`, `user:…`, `flow:…`, `queue:…`, a SIP
-- URI). The holiday target falls back to the closed one.
ALTER TABLE line_bindings ADD COLUMN IF NOT EXISTS schedule_id TEXT;
ALTER TABLE line_bindings ADD COLUMN IF NOT EXISTS closed_target TEXT;
ALTER TABLE line_bindings ADD COLUMN IF NOT EXISTS holiday_target TEXT;

-- A database-configured queue's hours, and where a caller goes when closed.
ALTER TABLE queue_configs ADD COLUMN IF NOT EXISTS schedule_id TEXT;
ALTER TABLE queue_configs ADD COLUMN IF NOT EXISTS closed_target TEXT;
