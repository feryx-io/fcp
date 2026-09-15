-- Legal holds (docs/COMPLIANCE_FOLLOWUPS.md §5). The rules are
-- fcp_core::legal_hold::LegalHold::covers in SQL: retention leaves held rows
-- out of its deletes, and a trigger on each governed table refuses any
-- delete that would remove a held row, whatever path it came by.

CREATE TABLE IF NOT EXISTS legal_holds (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    matter TEXT NOT NULL,
    reason TEXT,
    held_from TIMESTAMPTZ,
    held_to TIMESTAMPTZ,
    subjects JSONB NOT NULL DEFAULT '[]',
    subject_keys TEXT[] NOT NULL DEFAULT '{}',
    call_ids TEXT[] NOT NULL DEFAULT '{}',
    classes TEXT[] NOT NULL DEFAULT '{}',
    created_by TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    released_by TEXT,
    released_at TIMESTAMPTZ,
    release_reason TEXT
);
CREATE INDEX IF NOT EXISTS idx_legal_holds_active ON legal_holds (tenant_id) WHERE released_at IS NULL;

-- fcp_core::subject::keys, spelling for spelling.
CREATE OR REPLACE FUNCTION fcp_subject_keys(identifier TEXT) RETURNS TEXT[]
LANGUAGE plpgsql IMMUTABLE AS $$
DECLARE
    s TEXT;
    usr TEXT := '';
    host TEXT := '';
    digits TEXT;
    out TEXT[] := '{}';
BEGIN
    IF identifier IS NULL THEN
        RETURN out;
    END IF;
    s := btrim(identifier);
    s := COALESCE(substring(s FROM '<([^>]*)>'), s);
    s := regexp_replace(s, '^(sips|sip|tel):', '', 'i');
    IF position('@' IN s) > 0 THEN
        usr := substring(s FOR position('@' IN s) - 1);
        host := substring(s FROM position('@' IN s) + 1);
    ELSE
        usr := s;
    END IF;
    usr := lower(btrim(split_part(split_part(usr, ';', 1), '?', 1)));
    host := lower(btrim(split_part(split_part(split_part(host, ';', 1), '?', 1), ':', 1)));
    digits := regexp_replace(usr, '[^0-9]', '', 'g');
    IF usr <> '' AND digits <> '' AND usr ~ '^[0-9+.() -]+$' THEN
        out := out || ('num:' || digits);
        IF length(digits) >= 10 THEN
            out := out || ('tail:' || right(digits, 10));
        END IF;
    ELSIF usr <> '' THEN
        out := out || ('user:' || usr);
    END IF;
    IF usr <> '' AND host <> '' THEN
        out := out || ('aor:' || usr || '@' || host);
    END IF;
    RETURN out;
END
$$;

-- The first active hold of `tenant` covering an item: its class, the
-- instant it is dated by, its subject keys (NULL when they cannot be known,
-- which a hold with subjects treats as covered) and its call.
CREATE OR REPLACE FUNCTION fcp_hold_matches(
    tenant TEXT, class TEXT, at TIMESTAMPTZ, keys TEXT[], call TEXT
) RETURNS TEXT
LANGUAGE sql STABLE AS $$
    SELECT h.id FROM legal_holds h
    WHERE h.tenant_id = tenant
      AND h.released_at IS NULL
      AND (cardinality(h.classes) = 0 OR class = ANY (h.classes))
      AND (h.held_from IS NULL OR at >= h.held_from)
      AND (h.held_to IS NULL OR at < h.held_to)
      AND (
            (cardinality(h.subject_keys) = 0 AND cardinality(h.call_ids) = 0)
         OR (cardinality(h.subject_keys) > 0 AND (keys IS NULL OR keys && h.subject_keys))
         OR (call IS NOT NULL AND call = ANY (h.call_ids))
      )
    ORDER BY h.created_at, h.id
    LIMIT 1
$$;

-- Whether one hold covers an item (the same rules, for counting what a
-- hold holds).
CREATE OR REPLACE FUNCTION fcp_hold_covers(
    hold TEXT, class TEXT, at TIMESTAMPTZ, keys TEXT[], call TEXT
) RETURNS BOOLEAN
LANGUAGE sql STABLE AS $$
    SELECT EXISTS (
        SELECT 1 FROM legal_holds h
        WHERE h.id = hold
          AND h.released_at IS NULL
          AND (cardinality(h.classes) = 0 OR class = ANY (h.classes))
          AND (h.held_from IS NULL OR at >= h.held_from)
          AND (h.held_to IS NULL OR at < h.held_to)
          AND (
                (cardinality(h.subject_keys) = 0 AND cardinality(h.call_ids) = 0)
             OR (cardinality(h.subject_keys) > 0 AND (keys IS NULL OR keys && h.subject_keys))
             OR (call IS NOT NULL AND call = ANY (h.call_ids))
          ))
$$;

-- The subject keys of everyone a conference recording names.
CREATE OR REPLACE FUNCTION fcp_participant_keys(participants JSONB) RETURNS TEXT[]
LANGUAGE plpgsql IMMUTABLE AS $$
DECLARE
    k TEXT[] := '{}';
    p JSONB;
BEGIN
    FOR p IN SELECT jsonb_array_elements(COALESCE(participants, '[]')) LOOP
        k := k || fcp_subject_keys(p->>'uri') || fcp_subject_keys(p->>'display_name');
    END LOOP;
    RETURN k;
END $$;

CREATE OR REPLACE FUNCTION fcp_tenant_has_holds(tenant TEXT) RETURNS BOOLEAN
LANGUAGE sql STABLE AS $$
    SELECT EXISTS (SELECT 1 FROM legal_holds WHERE tenant_id = tenant AND released_at IS NULL)
$$;

-- The subject keys of a call, from its stored record; NULL without one.
CREATE OR REPLACE FUNCTION fcp_call_keys(call TEXT) RETURNS TEXT[]
LANGUAGE sql STABLE AS $$
    SELECT fcp_subject_keys(c.from_uri) || fcp_subject_keys(c.to_uri)
    FROM cdrs c WHERE c.call_id = call
    ORDER BY (c.kind = 'call') DESC
    LIMIT 1
$$;

-- One function per class: NULL, or the hold that covers the row. Keys are
-- worked out only when the tenant holds anything.
CREATE OR REPLACE FUNCTION fcp_cdr_hold(tenant TEXT, at TIMESTAMPTZ, from_uri TEXT, to_uri TEXT, call TEXT)
RETURNS TEXT LANGUAGE plpgsql STABLE AS $$
BEGIN
    IF NOT fcp_tenant_has_holds(tenant) THEN RETURN NULL; END IF;
    RETURN fcp_hold_matches(tenant, 'cdr', at, fcp_subject_keys(from_uri) || fcp_subject_keys(to_uri), call);
END $$;

CREATE OR REPLACE FUNCTION fcp_sip_trace_hold(tenant TEXT, at TIMESTAMPTZ, from_uri TEXT, to_uri TEXT, call TEXT)
RETURNS TEXT LANGUAGE plpgsql STABLE AS $$
BEGIN
    IF NOT fcp_tenant_has_holds(tenant) THEN RETURN NULL; END IF;
    RETURN fcp_hold_matches(tenant, 'sip_trace', at, fcp_subject_keys(from_uri) || fcp_subject_keys(to_uri), call);
END $$;

CREATE OR REPLACE FUNCTION fcp_call_recording_hold(tenant TEXT, at TIMESTAMPTZ, call TEXT)
RETURNS TEXT LANGUAGE plpgsql STABLE AS $$
BEGIN
    IF NOT fcp_tenant_has_holds(tenant) THEN RETURN NULL; END IF;
    RETURN fcp_hold_matches(tenant, 'call_recording', at, fcp_call_keys(call), call);
END $$;

CREATE OR REPLACE FUNCTION fcp_flow_trace_hold(tenant TEXT, at TIMESTAMPTZ, call TEXT)
RETURNS TEXT LANGUAGE plpgsql STABLE AS $$
BEGIN
    IF tenant IS NULL OR NOT fcp_tenant_has_holds(tenant) THEN RETURN NULL; END IF;
    RETURN fcp_hold_matches(tenant, 'flow_trace', at, fcp_call_keys(call), call);
END $$;

CREATE OR REPLACE FUNCTION fcp_conference_recording_hold(tenant TEXT, at TIMESTAMPTZ, participants JSONB)
RETURNS TEXT LANGUAGE plpgsql STABLE AS $$
BEGIN
    IF NOT fcp_tenant_has_holds(tenant) THEN RETURN NULL; END IF;
    RETURN fcp_hold_matches(tenant, 'conference_recording', at, fcp_participant_keys(participants), NULL);
END $$;

CREATE OR REPLACE FUNCTION fcp_voicemail_hold(tenant TEXT, at TIMESTAMPTZ, caller TEXT, mailbox TEXT)
RETURNS TEXT LANGUAGE plpgsql STABLE AS $$
BEGIN
    IF tenant IS NULL OR NOT fcp_tenant_has_holds(tenant) THEN RETURN NULL; END IF;
    RETURN fcp_hold_matches(tenant, 'voicemail', at, fcp_subject_keys(caller) || ('mailbox:' || mailbox), NULL);
END $$;

CREATE OR REPLACE FUNCTION fcp_audit_event_hold(tenant TEXT, at TIMESTAMPTZ, actor TEXT)
RETURNS TEXT LANGUAGE plpgsql STABLE AS $$
BEGIN
    IF tenant IS NULL OR NOT fcp_tenant_has_holds(tenant) THEN RETURN NULL; END IF;
    RETURN fcp_hold_matches(tenant, 'audit_event', at, fcp_subject_keys(actor), NULL);
END $$;

-- A trace's tenant, or its call's when it was captured before anyone knew.
CREATE OR REPLACE FUNCTION fcp_trace_tenant(tenant TEXT, call TEXT) RETURNS TEXT
LANGUAGE sql STABLE AS $$
    SELECT CASE WHEN tenant IS NOT NULL AND tenant <> '_system' THEN tenant
                ELSE (SELECT c.tenant_id FROM cdrs c WHERE c.call_id = call LIMIT 1) END
$$;

-- ── The backstop: a delete that would remove a held row is refused ─────
CREATE OR REPLACE FUNCTION fcp_refuse_held(hold TEXT) RETURNS VOID
LANGUAGE plpgsql AS $$
BEGIN
    IF hold IS NOT NULL THEN
        RAISE EXCEPTION 'held by legal hold %', hold
            USING ERRCODE = 'HLD01', DETAIL = hold;
    END IF;
END $$;

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
    END CASE;
    PERFORM fcp_refuse_held(hold);
    RETURN NULL;
END
$$;

DROP TRIGGER IF EXISTS cdrs_guard_held ON cdrs;
CREATE TRIGGER cdrs_guard_held AFTER DELETE ON cdrs
    REFERENCING OLD TABLE AS gone FOR EACH STATEMENT
    EXECUTE FUNCTION fcp_guard_held_deletion('cdr');
DROP TRIGGER IF EXISTS sip_traces_guard_held ON sip_trace_messages;
CREATE TRIGGER sip_traces_guard_held AFTER DELETE ON sip_trace_messages
    REFERENCING OLD TABLE AS gone FOR EACH STATEMENT
    EXECUTE FUNCTION fcp_guard_held_deletion('sip_trace');
DROP TRIGGER IF EXISTS flow_traces_guard_held ON flow_execution_traces;
CREATE TRIGGER flow_traces_guard_held AFTER DELETE ON flow_execution_traces
    REFERENCING OLD TABLE AS gone FOR EACH STATEMENT
    EXECUTE FUNCTION fcp_guard_held_deletion('flow_trace');
DROP TRIGGER IF EXISTS audit_log_guard_held ON admin_audit_log;
CREATE TRIGGER audit_log_guard_held AFTER DELETE ON admin_audit_log
    REFERENCING OLD TABLE AS gone FOR EACH STATEMENT
    EXECUTE FUNCTION fcp_guard_held_deletion('audit_event');
DROP TRIGGER IF EXISTS call_recordings_guard_held ON call_recordings;
CREATE TRIGGER call_recordings_guard_held AFTER DELETE ON call_recordings
    REFERENCING OLD TABLE AS gone FOR EACH STATEMENT
    EXECUTE FUNCTION fcp_guard_held_deletion('call_recording');
DROP TRIGGER IF EXISTS conference_recordings_guard_held ON conference_recordings;
CREATE TRIGGER conference_recordings_guard_held AFTER DELETE ON conference_recordings
    REFERENCING OLD TABLE AS gone FOR EACH STATEMENT
    EXECUTE FUNCTION fcp_guard_held_deletion('conference_recording');
DROP TRIGGER IF EXISTS voicemail_messages_guard_held ON voicemail_messages;
CREATE TRIGGER voicemail_messages_guard_held AFTER DELETE ON voicemail_messages
    REFERENCING OLD TABLE AS gone FOR EACH STATEMENT
    EXECUTE FUNCTION fcp_guard_held_deletion('voicemail');

-- A tenant that holds anything is not deleted.
CREATE OR REPLACE FUNCTION fcp_guard_tenant_deletion() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    hold TEXT;
BEGIN
    SELECT id INTO hold FROM legal_holds WHERE tenant_id = OLD.id AND released_at IS NULL LIMIT 1;
    PERFORM fcp_refuse_held(hold);
    RETURN OLD;
END $$;
DROP TRIGGER IF EXISTS tenants_guard_held ON tenants;
CREATE TRIGGER tenants_guard_held BEFORE DELETE ON tenants
    FOR EACH ROW EXECUTE FUNCTION fcp_guard_tenant_deletion();

-- ── Held archive days: an object lock where the bucket supports one ────
ALTER TABLE cdr_archives ADD COLUMN IF NOT EXISTS locked BOOLEAN NOT NULL DEFAULT false;

-- Retention runs count what holds kept, by class.
ALTER TABLE retention_runs ADD COLUMN IF NOT EXISTS held JSONB NOT NULL DEFAULT '{}';
