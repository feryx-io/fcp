-- Webinars W4 (docs/WEBINARS.md §2.6): a room's sittings.
--
-- One row per run of a room on a node, from its first caller to its last
-- (a webinar's lobby included). `id` is the `session_id` of the sitting's
-- questions (`webinar_questions`) and chat (`conference_chat`). `kind` is
-- the meeting's when it began. A webinar's `live_at` is when a host took
-- it live (NULL if it never went live); a meeting's is its start.
-- `ended_by` is a participant id, `host`, `api` or `stage_left` for a
-- webinar a host or its empty stage ended, NULL when its last caller left.
CREATE TABLE IF NOT EXISTS conference_sittings (
    id          TEXT PRIMARY KEY,
    tenant_id   TEXT NOT NULL,
    meeting_id  TEXT,
    room_number TEXT NOT NULL,
    kind        TEXT NOT NULL DEFAULT 'meeting',
    started_at  TIMESTAMPTZ NOT NULL,
    live_at     TIMESTAMPTZ,
    ended_at    TIMESTAMPTZ,
    ended_by    TEXT
);

CREATE INDEX IF NOT EXISTS conference_sittings_meeting
    ON conference_sittings (tenant_id, meeting_id, started_at DESC);
CREATE INDEX IF NOT EXISTS conference_sittings_room
    ON conference_sittings (tenant_id, room_number, started_at DESC);
