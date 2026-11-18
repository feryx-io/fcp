-- Webinars W3b (docs/WEBINARS.md §2.5): chat in a room kept under the
-- tenant's retention policy, held by a legal hold, found by a subject
-- export and erased by a subject erasure (docs/COMPLIANCE_FOLLOWUPS.md).

ALTER TABLE compliance_policies
    ADD COLUMN IF NOT EXISTS conference_chat_retention_days INTEGER;
ALTER TABLE retention_runs
    ADD COLUMN IF NOT EXISTS conference_chat_deleted BIGINT NOT NULL DEFAULT 0;

-- ── Legal holds: a message is held by its sender's keys ───────────────
-- Where they called from (their From URI) and the name the room showed.
CREATE OR REPLACE FUNCTION fcp_conference_chat_keys(address TEXT, name TEXT) RETURNS TEXT[]
LANGUAGE sql IMMUTABLE AS $$
    SELECT COALESCE(fcp_subject_keys(address), '{}') || COALESCE(fcp_subject_keys(name), '{}')
$$;

CREATE OR REPLACE FUNCTION fcp_conference_chat_hold(tenant TEXT, at TIMESTAMPTZ, address TEXT, name TEXT)
RETURNS TEXT LANGUAGE plpgsql STABLE AS $$
BEGIN
    IF NOT fcp_tenant_has_holds(tenant) THEN RETURN NULL; END IF;
    RETURN fcp_hold_matches(tenant, 'conference_chat', at, fcp_conference_chat_keys(address, name), NULL);
END $$;

-- Migration 132's guard, with chat added.
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
    WHEN 'conference_chat' THEN
        SELECT h INTO hold FROM (SELECT fcp_conference_chat_hold(tenant_id, sent_at, sender_address, sender_name) AS h FROM gone) x
        WHERE h IS NOT NULL LIMIT 1;
    END CASE;
    PERFORM fcp_refuse_held(hold);
    RETURN NULL;
END
$$;

DROP TRIGGER IF EXISTS conference_chat_guard_held ON conference_chat;
CREATE TRIGGER conference_chat_guard_held AFTER DELETE ON conference_chat
    REFERENCING OLD TABLE AS gone FOR EACH STATEMENT
    EXECUTE FUNCTION fcp_guard_held_deletion('conference_chat');

-- ── The deletion log: what went, counted by tenant and time ───────────
DROP TRIGGER IF EXISTS conference_chat_log_deletion ON conference_chat;
CREATE TRIGGER conference_chat_log_deletion AFTER DELETE ON conference_chat
    REFERENCING OLD TABLE AS gone FOR EACH STATEMENT
    EXECUTE FUNCTION fcp_log_bulk_deletion('conference_chat', 'sent_at');
