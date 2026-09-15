-- Compliance foundations (docs/COMPLIANCE_FOLLOWUPS.md §4, §7): every
-- recording a row, tenancy on the traces and voicemail messages, a log of
-- what each deletion removed, and a queue of files to remove, both written
-- in the same transaction as the rows they follow.

-- ── The deletion log ────────────────────────────────────────────────────
-- One row per item for recordings and voicemail messages, one per tenant
-- per statement for bulk classes (records, traces, audit events). The
-- deleting code names the cause and actor with
-- set_config('fcp.deletion_cause' / 'fcp.deletion_actor', …, true); a
-- voicemail message deleted by its mailbox's cascade is logged as a cascade,
-- under the actor who deleted what it hung off.
CREATE TABLE IF NOT EXISTS deletion_log (
    id BIGSERIAL PRIMARY KEY,
    tenant_id TEXT,
    class TEXT NOT NULL,
    item TEXT NOT NULL,
    detail JSONB,
    cause TEXT NOT NULL,
    actor TEXT NOT NULL,
    deleted_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_deletion_log_tenant ON deletion_log (tenant_id, deleted_at DESC);
CREATE INDEX IF NOT EXISTS idx_deletion_log_at ON deletion_log (deleted_at);

CREATE OR REPLACE FUNCTION fcp_deletion_cause() RETURNS TEXT
LANGUAGE sql STABLE AS $$
    SELECT COALESCE(NULLIF(current_setting('fcp.deletion_cause', true), ''), 'unattributed')
$$;

CREATE OR REPLACE FUNCTION fcp_deletion_actor() RETURNS TEXT
LANGUAGE sql STABLE AS $$
    SELECT COALESCE(NULLIF(current_setting('fcp.deletion_actor', true), ''), 'unknown')
$$;

-- ── Files to remove ─────────────────────────────────────────────────────
-- A row naming a file is deleted; the file goes on this queue in the same
-- transaction, and fcp-admin's reaper removes it once no live row names
-- the path (a voicemail greeting is rewritten under the same path).
CREATE TABLE IF NOT EXISTS pending_file_deletions (
    id BIGSERIAL PRIMARY KEY,
    tenant_id TEXT,
    class TEXT NOT NULL,
    path TEXT NOT NULL,
    queued_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    attempts INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_pending_file_deletions_due ON pending_file_deletions (next_attempt_at);

-- ── Call recordings ─────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS call_recordings (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    call_id TEXT NOT NULL,
    source TEXT NOT NULL,
    node_id TEXT,
    path TEXT NOT NULL,
    format TEXT NOT NULL,
    consent TEXT NOT NULL DEFAULT 'not_required',
    started_at TIMESTAMPTZ NOT NULL,
    ended_at TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS idx_call_recordings_tenant_started ON call_recordings (tenant_id, started_at);
CREATE INDEX IF NOT EXISTS idx_call_recordings_call ON call_recordings (call_id);
CREATE INDEX IF NOT EXISTS idx_call_recordings_path ON call_recordings (path);

-- The recordings the records already name.
INSERT INTO call_recordings (id, tenant_id, call_id, source, node_id, path, format, started_at, ended_at)
SELECT 'rec-' || md5(c.cdr_id::text)::uuid::text, c.tenant_id, c.call_id,
       CASE WHEN c.kind = 'queue' THEN 'queue' ELSE 'legacy' END,
       c.node_id,
       regexp_replace(c.recording_url, '^file://', ''),
       lower(COALESCE(substring(c.recording_url FROM '\.([A-Za-z0-9]+)$'), 'wav')),
       c.start_time, c.end_time
FROM cdrs c
WHERE c.recording_url IS NOT NULL AND c.recording_url <> ''
ON CONFLICT (id) DO NOTHING;

-- ── Conference recordings: kind, participant, who was in the room ──────
ALTER TABLE conference_recordings ADD COLUMN IF NOT EXISTS kind TEXT NOT NULL DEFAULT 'room';
ALTER TABLE conference_recordings ADD COLUMN IF NOT EXISTS participant_id TEXT;
ALTER TABLE conference_recordings ADD COLUMN IF NOT EXISTS participant_list JSONB NOT NULL DEFAULT '[]';

-- ── Tenancy on voicemail messages and flow traces ──────────────────────
ALTER TABLE voicemail_messages ADD COLUMN IF NOT EXISTS tenant_id TEXT;
ALTER TABLE voicemail_messages ADD COLUMN IF NOT EXISTS deleted_at TIMESTAMPTZ;
UPDATE voicemail_messages m SET tenant_id = mb.tenant_id
FROM voicemail_mailboxes mb WHERE mb.id = m.mailbox_id AND m.tenant_id IS NULL;
UPDATE voicemail_messages SET deleted_at = now() WHERE status = 'deleted' AND deleted_at IS NULL;
CREATE INDEX IF NOT EXISTS idx_voicemail_messages_tenant_created ON voicemail_messages (tenant_id, created_at);
CREATE INDEX IF NOT EXISTS idx_voicemail_messages_deleted ON voicemail_messages (deleted_at) WHERE deleted_at IS NOT NULL;

CREATE OR REPLACE FUNCTION voicemail_message_bookkeeping() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.tenant_id IS NULL THEN
        SELECT tenant_id INTO NEW.tenant_id FROM voicemail_mailboxes WHERE id = NEW.mailbox_id;
    END IF;
    IF NEW.status = 'deleted' AND NEW.deleted_at IS NULL THEN
        NEW.deleted_at := now();
    ELSIF NEW.status <> 'deleted' THEN
        NEW.deleted_at := NULL;
    END IF;
    RETURN NEW;
END
$$;
DROP TRIGGER IF EXISTS voicemail_messages_bookkeeping ON voicemail_messages;
CREATE TRIGGER voicemail_messages_bookkeeping
    BEFORE INSERT OR UPDATE ON voicemail_messages
    FOR EACH ROW EXECUTE FUNCTION voicemail_message_bookkeeping();

ALTER TABLE flow_execution_traces ADD COLUMN IF NOT EXISTS tenant_id TEXT;
UPDATE flow_execution_traces t SET tenant_id = c.tenant_id
FROM cdrs c WHERE c.call_id = t.call_id AND t.tenant_id IS NULL;
CREATE INDEX IF NOT EXISTS idx_flow_traces_tenant_ts ON flow_execution_traces (tenant_id, timestamp);

-- ── Voicemail retention on the policy ──────────────────────────────────
ALTER TABLE compliance_policies ADD COLUMN IF NOT EXISTS voicemail_retention_days INTEGER;

-- ── The triggers that log and queue ────────────────────────────────────
CREATE OR REPLACE FUNCTION fcp_log_bulk_deletion() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    -- TG_ARGV[0]: class; TG_ARGV[1]: the timestamp column.
    EXECUTE format(
        'INSERT INTO deletion_log (tenant_id, class, item, detail, cause, actor)
         SELECT tenant_id, %L,
                min(%2$I)::text || '' .. '' || max(%2$I)::text,
                jsonb_build_object(''count'', count(*), ''from'', min(%2$I), ''to'', max(%2$I)),
                fcp_deletion_cause(), fcp_deletion_actor()
         FROM gone GROUP BY tenant_id',
        TG_ARGV[0], TG_ARGV[1]);
    RETURN NULL;
END
$$;

DROP TRIGGER IF EXISTS cdrs_log_deletion ON cdrs;
CREATE TRIGGER cdrs_log_deletion AFTER DELETE ON cdrs
    REFERENCING OLD TABLE AS gone FOR EACH STATEMENT
    EXECUTE FUNCTION fcp_log_bulk_deletion('cdr', 'start_time');

DROP TRIGGER IF EXISTS sip_traces_log_deletion ON sip_trace_messages;
CREATE TRIGGER sip_traces_log_deletion AFTER DELETE ON sip_trace_messages
    REFERENCING OLD TABLE AS gone FOR EACH STATEMENT
    EXECUTE FUNCTION fcp_log_bulk_deletion('sip_trace', 'timestamp');

DROP TRIGGER IF EXISTS flow_traces_log_deletion ON flow_execution_traces;
CREATE TRIGGER flow_traces_log_deletion AFTER DELETE ON flow_execution_traces
    REFERENCING OLD TABLE AS gone FOR EACH STATEMENT
    EXECUTE FUNCTION fcp_log_bulk_deletion('flow_trace', 'timestamp');

DROP TRIGGER IF EXISTS audit_log_log_deletion ON admin_audit_log;
CREATE TRIGGER audit_log_log_deletion AFTER DELETE ON admin_audit_log
    REFERENCING OLD TABLE AS gone FOR EACH STATEMENT
    EXECUTE FUNCTION fcp_log_bulk_deletion('audit_event', 'occurred_at');

CREATE OR REPLACE FUNCTION fcp_log_conference_recording_deletion() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    -- The holding node removes the file itself (its sweep and the API's
    -- delete both go through it), so nothing is queued.
    INSERT INTO deletion_log (tenant_id, class, item, detail, cause, actor)
    SELECT tenant_id, 'conference_recording', id,
           jsonb_build_object('room_number', room_number, 'node_id', node_id,
                              'path', path, 'kind', kind, 'started_at', started_at),
           fcp_deletion_cause(), fcp_deletion_actor()
    FROM gone;
    RETURN NULL;
END
$$;
DROP TRIGGER IF EXISTS conference_recordings_log_deletion ON conference_recordings;
CREATE TRIGGER conference_recordings_log_deletion AFTER DELETE ON conference_recordings
    REFERENCING OLD TABLE AS gone FOR EACH STATEMENT
    EXECUTE FUNCTION fcp_log_conference_recording_deletion();

CREATE OR REPLACE FUNCTION fcp_log_call_recording_deletion() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    INSERT INTO deletion_log (tenant_id, class, item, detail, cause, actor)
    SELECT tenant_id, 'call_recording', id,
           jsonb_build_object('call_id', call_id, 'path', path, 'source', source,
                              'started_at', started_at),
           fcp_deletion_cause(), fcp_deletion_actor()
    FROM gone;
    INSERT INTO pending_file_deletions (tenant_id, class, path)
    SELECT tenant_id, 'call_recording', path FROM gone;
    -- The record no longer names a recording that is gone.
    UPDATE cdrs c SET recording_url = NULL
    FROM gone g
    WHERE c.call_id = g.call_id
      AND regexp_replace(c.recording_url, '^file://', '') = g.path;
    RETURN NULL;
END
$$;
DROP TRIGGER IF EXISTS call_recordings_log_deletion ON call_recordings;
CREATE TRIGGER call_recordings_log_deletion AFTER DELETE ON call_recordings
    REFERENCING OLD TABLE AS gone FOR EACH STATEMENT
    EXECUTE FUNCTION fcp_log_call_recording_deletion();

CREATE OR REPLACE FUNCTION fcp_log_voicemail_message_deletion() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    -- A statement trigger fires after the whole statement, so a cascade
    -- shows as the mailbox already gone.
    INSERT INTO deletion_log (tenant_id, class, item, detail, cause, actor)
    SELECT g.tenant_id, 'voicemail', g.id,
           jsonb_build_object('mailbox_id', g.mailbox_id, 'path', g.recording_path,
                              'created_at', g.created_at),
           CASE WHEN EXISTS (SELECT 1 FROM voicemail_mailboxes mb WHERE mb.id = g.mailbox_id)
                THEN fcp_deletion_cause() ELSE 'cascade' END,
           fcp_deletion_actor()
    FROM gone g;
    INSERT INTO pending_file_deletions (tenant_id, class, path)
    SELECT tenant_id, 'voicemail', recording_path FROM gone WHERE recording_path <> '';
    RETURN NULL;
END
$$;
DROP TRIGGER IF EXISTS voicemail_messages_log_deletion ON voicemail_messages;
CREATE TRIGGER voicemail_messages_log_deletion AFTER DELETE ON voicemail_messages
    REFERENCING OLD TABLE AS gone FOR EACH STATEMENT
    EXECUTE FUNCTION fcp_log_voicemail_message_deletion();

-- A greeting row is rewritten on every mailbox save, so its deletion is
-- not logged; its file is queued, and the reaper keeps it while a row
-- still names it.
CREATE OR REPLACE FUNCTION fcp_queue_greeting_file() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    INSERT INTO pending_file_deletions (tenant_id, class, path)
    SELECT mb.tenant_id, 'voicemail_greeting', g.audio_path
    FROM gone g LEFT JOIN voicemail_mailboxes mb ON mb.id = g.mailbox_id
    WHERE g.audio_path <> '';
    RETURN NULL;
END
$$;
DROP TRIGGER IF EXISTS voicemail_greetings_queue_file ON voicemail_greetings;
CREATE TRIGGER voicemail_greetings_queue_file AFTER DELETE ON voicemail_greetings
    REFERENCING OLD TABLE AS gone FOR EACH STATEMENT
    EXECUTE FUNCTION fcp_queue_greeting_file();

-- A call's stored record closes the recordings still open on it.
CREATE OR REPLACE FUNCTION cdr_archive_note_late() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    walked DATE;
BEGIN
    UPDATE call_recordings r SET ended_at = COALESCE(n.end_time, n.start_time)
    FROM new_records n
    WHERE r.call_id = n.call_id AND r.ended_at IS NULL;

    SELECT next_day INTO walked FROM cdr_archive_state FOR KEY SHARE;
    IF walked IS NULL THEN
        RETURN NULL;
    END IF;
    INSERT INTO cdr_archive_late (tenant_id, day)
    SELECT DISTINCT tenant_id, (start_time AT TIME ZONE 'UTC')::date
    FROM new_records
    WHERE (start_time AT TIME ZONE 'UTC')::date <= walked;
    RETURN NULL;
END
$$;
ALTER TABLE retention_runs ADD COLUMN IF NOT EXISTS voicemail_messages_deleted BIGINT NOT NULL DEFAULT 0;
