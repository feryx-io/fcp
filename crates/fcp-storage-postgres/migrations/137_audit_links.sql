-- A request's audit row and what it deleted, tied by the request's id
-- (#342): the `X-Request-Id` the API gives every request is kept on its
-- audit row, and the deletion log's triggers take it, as they take the
-- cause and actor, from the transaction's settings (`fcp.api_request`).
ALTER TABLE admin_audit_log ADD COLUMN IF NOT EXISTS api_request_id TEXT;
CREATE INDEX IF NOT EXISTS idx_admin_audit_log_api_request ON admin_audit_log (api_request_id)
    WHERE api_request_id IS NOT NULL;

ALTER TABLE deletion_log ADD COLUMN IF NOT EXISTS api_request_id TEXT;
ALTER TABLE deletion_log
    ALTER COLUMN api_request_id SET DEFAULT NULLIF(current_setting('fcp.api_request', true), '');
CREATE INDEX IF NOT EXISTS idx_deletion_log_api_request ON deletion_log (api_request_id)
    WHERE api_request_id IS NOT NULL;

-- How the actor reached what they deleted, beside who they are: the phone
-- a mailbox's owner called from (`fcp.deletion_via`).
ALTER TABLE deletion_log ADD COLUMN IF NOT EXISTS via TEXT;
ALTER TABLE deletion_log
    ALTER COLUMN via SET DEFAULT NULLIF(current_setting('fcp.deletion_via', true), '');
