-- Webinars W4.2 (docs/WEBINARS.md §2.5–2.6): a webinar's questions and
-- registrations kept under the tenant's retention policy, held by a legal
-- hold, found by a subject export and erased by a subject erasure
-- (docs/COMPLIANCE_FOLLOWUPS.md), as chat is (migration 144).

-- Where the asker called from (their From URI): what a subject request
-- and a hold find them by, as chat's `sender_address`. NULL for questions
-- asked before this migration, and through the API without a call.
ALTER TABLE webinar_questions
    ADD COLUMN IF NOT EXISTS asker_address TEXT;

-- One window each: a question ages by when it was asked, a registration
-- by when it was made.
ALTER TABLE compliance_policies
    ADD COLUMN IF NOT EXISTS webinar_question_retention_days INTEGER,
    ADD COLUMN IF NOT EXISTS webinar_registrant_retention_days INTEGER;
ALTER TABLE retention_runs
    ADD COLUMN IF NOT EXISTS webinar_questions_deleted BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS webinar_registrants_deleted BIGINT NOT NULL DEFAULT 0;

-- ── Legal holds ───────────────────────────────────────────────────────
-- A question is held by its asker's keys: where they called from and the
-- name the room showed (an anonymous question keeps no name, so only its
-- address finds it).
CREATE OR REPLACE FUNCTION fcp_webinar_question_hold(tenant TEXT, at TIMESTAMPTZ, address TEXT, name TEXT)
RETURNS TEXT LANGUAGE plpgsql STABLE AS $$
BEGIN
    IF NOT fcp_tenant_has_holds(tenant) THEN RETURN NULL; END IF;
    RETURN fcp_hold_matches(tenant, 'webinar_question', at, fcp_conference_chat_keys(address, name), NULL);
END $$;

-- A registration is held by the registrant's keys: their email and name.
CREATE OR REPLACE FUNCTION fcp_webinar_registrant_hold(tenant TEXT, at TIMESTAMPTZ, email TEXT, name TEXT)
RETURNS TEXT LANGUAGE plpgsql STABLE AS $$
BEGIN
    IF NOT fcp_tenant_has_holds(tenant) THEN RETURN NULL; END IF;
    RETURN fcp_hold_matches(tenant, 'webinar_registrant', at, fcp_conference_chat_keys(email, name), NULL);
END $$;

-- Migration 144's guard, with questions and registrations added.
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
    WHEN 'webinar_question' THEN
        SELECT h INTO hold FROM (SELECT fcp_webinar_question_hold(tenant_id, asked_at, asker_address, asker_name) AS h FROM gone) x
        WHERE h IS NOT NULL LIMIT 1;
    WHEN 'webinar_registrant' THEN
        SELECT h INTO hold FROM (SELECT fcp_webinar_registrant_hold(tenant_id, created_at, email, name) AS h FROM gone) x
        WHERE h IS NOT NULL LIMIT 1;
    END CASE;
    PERFORM fcp_refuse_held(hold);
    RETURN NULL;
END
$$;

DROP TRIGGER IF EXISTS webinar_questions_guard_held ON webinar_questions;
CREATE TRIGGER webinar_questions_guard_held AFTER DELETE ON webinar_questions
    REFERENCING OLD TABLE AS gone FOR EACH STATEMENT
    EXECUTE FUNCTION fcp_guard_held_deletion('webinar_question');

DROP TRIGGER IF EXISTS webinar_registrants_guard_held ON webinar_registrants;
CREATE TRIGGER webinar_registrants_guard_held AFTER DELETE ON webinar_registrants
    REFERENCING OLD TABLE AS gone FOR EACH STATEMENT
    EXECUTE FUNCTION fcp_guard_held_deletion('webinar_registrant');

-- ── The deletion log: what went, counted by tenant and time ───────────
DROP TRIGGER IF EXISTS webinar_questions_log_deletion ON webinar_questions;
CREATE TRIGGER webinar_questions_log_deletion AFTER DELETE ON webinar_questions
    REFERENCING OLD TABLE AS gone FOR EACH STATEMENT
    EXECUTE FUNCTION fcp_log_bulk_deletion('webinar_question', 'asked_at');

DROP TRIGGER IF EXISTS webinar_registrants_log_deletion ON webinar_registrants;
CREATE TRIGGER webinar_registrants_log_deletion AFTER DELETE ON webinar_registrants
    REFERENCING OLD TABLE AS gone FOR EACH STATEMENT
    EXECUTE FUNCTION fcp_log_bulk_deletion('webinar_registrant', 'created_at');
