-- Captures from people's apps (docs/CALL_DIAGNOSTICS.md §3): the tenant's
-- word on them, an administrator's allows, and the uploaded files' rows,
-- kept under retention and legal holds.

ALTER TABLE tenants
    ADD COLUMN IF NOT EXISTS diagnostics_json JSONB NOT NULL DEFAULT '{}'::jsonb;

-- Who may capture, until when: a user (every app they sign in to) or one
-- device. An allow is never longer than 30 days; the API sees to that.
CREATE TABLE IF NOT EXISTS capture_allows (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    user_id TEXT REFERENCES users(id) ON DELETE CASCADE,
    device_id TEXT REFERENCES devices(id) ON DELETE CASCADE,
    until TIMESTAMPTZ NOT NULL,
    reason TEXT,
    created_by TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK ((user_id IS NULL) <> (device_id IS NULL))
);
CREATE INDEX IF NOT EXISTS idx_capture_allows_tenant ON capture_allows (tenant_id, created_at);
CREATE INDEX IF NOT EXISTS idx_capture_allows_user ON capture_allows (user_id);
CREATE INDEX IF NOT EXISTS idx_capture_allows_device ON capture_allows (device_id);

-- An uploaded capture. The device is not a foreign key: a capture outlives
-- the app that took it, until retention.
CREATE TABLE IF NOT EXISTS device_captures (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    device_id TEXT NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    call_id TEXT,
    size BIGINT NOT NULL,
    sha256 TEXT NOT NULL,
    file_key TEXT NOT NULL,
    -- The user's subject keys at upload, for legal holds.
    subject_keys TEXT[] NOT NULL DEFAULT '{}',
    uploaded_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_device_captures_tenant ON device_captures (tenant_id, uploaded_at);
CREATE INDEX IF NOT EXISTS idx_device_captures_device ON device_captures (tenant_id, device_id);
CREATE INDEX IF NOT EXISTS idx_device_captures_expiry ON device_captures (expires_at);

-- Every removed capture's file, however its row went (an administrator,
-- retention, a purged user), queued for the admin to remove from the
-- captures' store.
CREATE TABLE IF NOT EXISTS device_capture_file_deletions (
    file_key TEXT PRIMARY KEY,
    tenant_id TEXT,
    queued_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    attempts INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_device_capture_file_deletions_due
    ON device_capture_file_deletions (next_attempt_at);

CREATE OR REPLACE FUNCTION fcp_queue_device_capture_files() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    INSERT INTO device_capture_file_deletions (file_key, tenant_id)
    SELECT file_key, tenant_id FROM gone
    ON CONFLICT (file_key) DO NOTHING;
    RETURN NULL;
END $$;
DROP TRIGGER IF EXISTS device_captures_queue_files ON device_captures;
CREATE TRIGGER device_captures_queue_files AFTER DELETE ON device_captures
    REFERENCING OLD TABLE AS gone FOR EACH STATEMENT
    EXECUTE FUNCTION fcp_queue_device_capture_files();

-- ── Legal holds: a capture is held by its user's keys or its call ──────
CREATE OR REPLACE FUNCTION fcp_device_capture_keys(keys TEXT[], call TEXT) RETURNS TEXT[]
LANGUAGE sql STABLE AS $$
    SELECT keys || COALESCE(fcp_call_keys(call), '{}')
$$;

CREATE OR REPLACE FUNCTION fcp_device_capture_hold(tenant TEXT, at TIMESTAMPTZ, keys TEXT[], call TEXT)
RETURNS TEXT LANGUAGE plpgsql STABLE AS $$
BEGIN
    IF NOT fcp_tenant_has_holds(tenant) THEN RETURN NULL; END IF;
    RETURN fcp_hold_matches(tenant, 'device_capture', at, fcp_device_capture_keys(keys, call), call);
END $$;

-- Migration 063's guard, with captures added.
CREATE OR REPLACE FUNCTION fcp_guard_held_deletion() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    hold TEXT;
BEGIN
    -- Nothing to look at while nobody holds anything. (Not by the rows'
    -- tenant: a trace captured before its tenant was known says `_system`.)
    IF NOT EXISTS (SELECT 1 FROM legal_holds h WHERE h.released_at IS NULL) THEN
        RETURN NULL;
    END IF;
    CASE TG_ARGV[0]
    WHEN 'cdr' THEN
        SELECT h INTO hold FROM (SELECT fcp_cdr_hold(tenant_id, start_time, from_uri, to_uri, call_id) AS h FROM gone) x
        WHERE h IS NOT NULL LIMIT 1;
    WHEN 'sip_trace' THEN
        SELECT h INTO hold FROM (
            SELECT fcp_sip_trace_hold(fcp_trace_tenant(tenant_id, call_id), timestamp, from_uri, to_uri, call_id) AS h
            FROM gone) x
        WHERE h IS NOT NULL LIMIT 1;
    WHEN 'flow_trace' THEN
        SELECT h INTO hold FROM (
            SELECT fcp_flow_trace_hold(fcp_trace_tenant(tenant_id, call_id), timestamp, call_id) AS h
            FROM gone) x
        WHERE h IS NOT NULL LIMIT 1;
    WHEN 'audit_event' THEN
        SELECT h INTO hold FROM (SELECT fcp_audit_event_hold(tenant_id, occurred_at, actor) AS h FROM gone) x
        WHERE h IS NOT NULL LIMIT 1;
    WHEN 'call_recording' THEN
        SELECT h INTO hold FROM (SELECT fcp_call_recording_hold(tenant_id, started_at, call_id) AS h FROM gone) x
        WHERE h IS NOT NULL LIMIT 1;
    WHEN 'conference_recording' THEN
        SELECT h INTO hold FROM (SELECT fcp_conference_recording_hold(tenant_id, started_at, participant_list) AS h FROM gone) x
        WHERE h IS NOT NULL LIMIT 1;
    WHEN 'voicemail' THEN
        SELECT h INTO hold FROM (SELECT fcp_voicemail_hold(tenant_id, created_at, caller, mailbox_id) AS h FROM gone) x
        WHERE h IS NOT NULL LIMIT 1;
    WHEN 'device_capture' THEN
        SELECT h INTO hold FROM (SELECT fcp_device_capture_hold(tenant_id, uploaded_at, subject_keys, call_id) AS h FROM gone) x
        WHERE h IS NOT NULL LIMIT 1;
    END CASE;
    PERFORM fcp_refuse_held(hold);
    RETURN NULL;
END
$$;

DROP TRIGGER IF EXISTS device_captures_guard_held ON device_captures;
CREATE TRIGGER device_captures_guard_held AFTER DELETE ON device_captures
    REFERENCING OLD TABLE AS gone FOR EACH STATEMENT
    EXECUTE FUNCTION fcp_guard_held_deletion('device_capture');
