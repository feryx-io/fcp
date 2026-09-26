-- Records up a site's link (docs/HYBRID_CONTROL_PLANE.md §3.3).
--
-- Every record a site ships carries a shipping sequence: one number, from
-- one sequence, taken as the row is written and again whenever it changes,
-- so the link reads "everything after my cursor" in order and idempotently.
-- Rows from before this migration have none and are not shipped; the
-- cloud's own rows take one too, which costs a sequence call and nothing
-- else. A record the cloud received from a site names the site.
CREATE SEQUENCE IF NOT EXISTS fcp_ship_seq;

CREATE OR REPLACE FUNCTION fcp_bump_ship_seq() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    NEW.ship_seq := nextval('fcp_ship_seq');
    RETURN NEW;
END
$$;

-- cdrs is partitioned by start_time: the column and the trigger reach every
-- partition, and the index is made on each.
ALTER TABLE cdrs ADD COLUMN IF NOT EXISTS ship_seq BIGINT;
ALTER TABLE cdrs ALTER COLUMN ship_seq SET DEFAULT nextval('fcp_ship_seq');
ALTER TABLE cdrs ADD COLUMN IF NOT EXISTS site_id TEXT;
CREATE INDEX IF NOT EXISTS idx_cdrs_ship_seq ON cdrs (ship_seq) WHERE ship_seq IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_cdrs_site ON cdrs (site_id, start_time DESC) WHERE site_id IS NOT NULL;

ALTER TABLE call_recordings ADD COLUMN IF NOT EXISTS ship_seq BIGINT;
ALTER TABLE call_recordings ALTER COLUMN ship_seq SET DEFAULT nextval('fcp_ship_seq');
ALTER TABLE call_recordings ADD COLUMN IF NOT EXISTS site_id TEXT;
CREATE INDEX IF NOT EXISTS idx_call_recordings_ship_seq ON call_recordings (ship_seq) WHERE ship_seq IS NOT NULL;
DROP TRIGGER IF EXISTS call_recordings_ship_seq ON call_recordings;
CREATE TRIGGER call_recordings_ship_seq BEFORE UPDATE ON call_recordings
    FOR EACH ROW EXECUTE FUNCTION fcp_bump_ship_seq();

ALTER TABLE conference_recordings ADD COLUMN IF NOT EXISTS ship_seq BIGINT;
ALTER TABLE conference_recordings ALTER COLUMN ship_seq SET DEFAULT nextval('fcp_ship_seq');
ALTER TABLE conference_recordings ADD COLUMN IF NOT EXISTS site_id TEXT;
CREATE INDEX IF NOT EXISTS idx_conference_recordings_ship_seq ON conference_recordings (ship_seq) WHERE ship_seq IS NOT NULL;
DROP TRIGGER IF EXISTS conference_recordings_ship_seq ON conference_recordings;
CREATE TRIGGER conference_recordings_ship_seq BEFORE UPDATE ON conference_recordings
    FOR EACH ROW EXECUTE FUNCTION fcp_bump_ship_seq();

ALTER TABLE voicemail_mailboxes ADD COLUMN IF NOT EXISTS ship_seq BIGINT;
ALTER TABLE voicemail_mailboxes ALTER COLUMN ship_seq SET DEFAULT nextval('fcp_ship_seq');
CREATE INDEX IF NOT EXISTS idx_voicemail_mailboxes_ship_seq ON voicemail_mailboxes (ship_seq) WHERE ship_seq IS NOT NULL;
DROP TRIGGER IF EXISTS voicemail_mailboxes_ship_seq ON voicemail_mailboxes;
CREATE TRIGGER voicemail_mailboxes_ship_seq BEFORE UPDATE ON voicemail_mailboxes
    FOR EACH ROW EXECUTE FUNCTION fcp_bump_ship_seq();

ALTER TABLE voicemail_messages ADD COLUMN IF NOT EXISTS ship_seq BIGINT;
ALTER TABLE voicemail_messages ALTER COLUMN ship_seq SET DEFAULT nextval('fcp_ship_seq');
ALTER TABLE voicemail_messages ADD COLUMN IF NOT EXISTS site_id TEXT;
CREATE INDEX IF NOT EXISTS idx_voicemail_messages_ship_seq ON voicemail_messages (ship_seq) WHERE ship_seq IS NOT NULL;
DROP TRIGGER IF EXISTS voicemail_messages_ship_seq ON voicemail_messages;
CREATE TRIGGER voicemail_messages_ship_seq BEFORE UPDATE ON voicemail_messages
    FOR EACH ROW EXECUTE FUNCTION fcp_bump_ship_seq();

-- A file a site holds is the site's to remove: the deletion queue says
-- which site, and the reaper asks it through its link rather than looking
-- for the file here.
ALTER TABLE pending_file_deletions ADD COLUMN IF NOT EXISTS site_id TEXT;

CREATE OR REPLACE FUNCTION fcp_log_call_recording_deletion() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    INSERT INTO deletion_log (tenant_id, class, item, detail, cause, actor)
    SELECT tenant_id, 'call_recording', id,
           jsonb_build_object('call_id', call_id, 'path', path, 'source', source,
                              'started_at', started_at, 'site_id', site_id),
           fcp_deletion_cause(), fcp_deletion_actor()
    FROM gone;
    INSERT INTO pending_file_deletions (tenant_id, class, path, site_id)
    SELECT tenant_id, 'call_recording', path, site_id FROM gone;
    -- The record no longer names a recording that is gone.
    UPDATE cdrs c SET recording_url = NULL
    FROM gone g
    WHERE c.call_id = g.call_id
      AND regexp_replace(c.recording_url, '^file://', '') = g.path;
    RETURN NULL;
END
$$;

CREATE OR REPLACE FUNCTION fcp_log_voicemail_message_deletion() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    -- A statement trigger fires after the whole statement, so a cascade
    -- shows as the mailbox already gone.
    INSERT INTO deletion_log (tenant_id, class, item, detail, cause, actor)
    SELECT g.tenant_id, 'voicemail', g.id,
           jsonb_build_object('mailbox_id', g.mailbox_id, 'path', g.recording_path,
                              'created_at', g.created_at, 'site_id', g.site_id),
           CASE WHEN EXISTS (SELECT 1 FROM voicemail_mailboxes mb WHERE mb.id = g.mailbox_id)
                THEN fcp_deletion_cause() ELSE 'cascade' END,
           fcp_deletion_actor()
    FROM gone g;
    INSERT INTO pending_file_deletions (tenant_id, class, path, site_id)
    SELECT tenant_id, 'voicemail', recording_path, site_id FROM gone WHERE recording_path <> '';
    RETURN NULL;
END
$$;

-- A conference recording's file is removed by the node holding it; one a
-- site holds is queued for the site instead.
CREATE OR REPLACE FUNCTION fcp_log_conference_recording_deletion() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    INSERT INTO deletion_log (tenant_id, class, item, detail, cause, actor)
    SELECT tenant_id, 'conference_recording', id,
           jsonb_build_object('room_number', room_number, 'node_id', node_id,
                              'path', path, 'kind', kind, 'started_at', started_at,
                              'site_id', site_id),
           fcp_deletion_cause(), fcp_deletion_actor()
    FROM gone;
    INSERT INTO pending_file_deletions (tenant_id, class, path, site_id)
    SELECT tenant_id, 'conference_recording', path, site_id FROM gone WHERE site_id IS NOT NULL;
    RETURN NULL;
END
$$;

-- What the cloud asks of a site (docs/HYBRID_CONTROL_PLANE.md §3.3): a
-- site opens no port, so a request waits here until the link collects it
-- and answers.
CREATE TABLE IF NOT EXISTS site_requests (
    id          TEXT PRIMARY KEY,
    site_id     TEXT NOT NULL REFERENCES sites (id) ON DELETE CASCADE,
    tenant_id   TEXT NOT NULL,
    request     JSONB NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL,
    taken_at    TIMESTAMPTZ,
    done_at     TIMESTAMPTZ,
    ok          BOOLEAN,
    detail      TEXT
);
CREATE INDEX IF NOT EXISTS idx_site_requests_pending ON site_requests (site_id, created_at)
    WHERE done_at IS NULL;
