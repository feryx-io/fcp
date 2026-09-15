-- Subject requests (docs/COMPLIANCE_FOLLOWUPS.md §6.2): a person's export or
-- erasure, filed by the tenant, run by fcp-admin against a deadline.

CREATE TABLE IF NOT EXISTS subject_requests (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    subjects JSONB NOT NULL,
    expansion JSONB,
    status TEXT NOT NULL,
    received_at TIMESTAMPTZ NOT NULL,
    due_at TIMESTAMPTZ NOT NULL,
    requested_by TEXT NOT NULL,
    reference TEXT,
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    report JSONB,
    bundle JSONB,
    error TEXT
);
CREATE INDEX IF NOT EXISTS idx_subject_requests_tenant ON subject_requests (tenant_id, received_at DESC);
CREATE INDEX IF NOT EXISTS idx_subject_requests_open ON subject_requests (status, received_at)
    WHERE status IN ('received', 'running');

ALTER TABLE compliance_policies
    ADD COLUMN IF NOT EXISTS subject_request_days INTEGER NOT NULL DEFAULT 30;
