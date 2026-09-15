-- Subject erasure and tenant deletion (docs/COMPLIANCE_FOLLOWUPS.md §6.4-6.6,
-- §4): a preview confirmed by a person, the pseudonym key each tenant's
-- records are rewritten with, the request every deletion it makes is logged
-- under, and whether a tenant's records are pseudonymised or deleted.

ALTER TABLE subject_requests ADD COLUMN IF NOT EXISTS confirmed_at TIMESTAMPTZ;
ALTER TABLE subject_requests ADD COLUMN IF NOT EXISTS confirmed_by TEXT;
ALTER TABLE subject_requests ADD COLUMN IF NOT EXISTS preview JSONB;
ALTER TABLE subject_requests ADD COLUMN IF NOT EXISTS erasure JSONB;
DROP INDEX IF EXISTS idx_subject_requests_open;
CREATE INDEX IF NOT EXISTS idx_subject_requests_open ON subject_requests (status, received_at)
    WHERE status IN ('received', 'running', 'preview', 'confirmed');

-- The request behind a deletion, set like its cause and actor. Added empty
-- and defaulted after, so existing rows are not rewritten.
ALTER TABLE deletion_log ADD COLUMN IF NOT EXISTS request_id TEXT;
ALTER TABLE deletion_log
    ALTER COLUMN request_id SET DEFAULT NULLIF(current_setting('fcp.deletion_request', true), '');
CREATE INDEX IF NOT EXISTS idx_deletion_log_request ON deletion_log (request_id)
    WHERE request_id IS NOT NULL;

-- Each tenant's pseudonym key, sealed with encryption at rest; `id` is the
-- tenant, as the re-encryption pass addresses rows by `id`.
CREATE TABLE IF NOT EXISTS subject_pseudonym_keys (
    id TEXT PRIMARY KEY,
    secret TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

ALTER TABLE compliance_policies
    ADD COLUMN IF NOT EXISTS cdr_erasure TEXT NOT NULL DEFAULT 'pseudonymise';
