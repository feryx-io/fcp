-- Webinars W4.2 (docs/WEBINARS.md §2.6): registration and attendance.
--
-- A webinar whose `registration` is set admits an attendee only by the
-- join token minted for their registration.
ALTER TABLE conference_meetings
    ADD COLUMN IF NOT EXISTS registration BOOLEAN NOT NULL DEFAULT false;

-- One person registered for a webinar. Their token names `id`; the room
-- admits it while `revoked_at` is NULL. Gone with the meeting.
CREATE TABLE IF NOT EXISTS webinar_registrants (
    id         TEXT PRIMARY KEY,
    tenant_id  TEXT NOT NULL,
    meeting_id TEXT NOT NULL REFERENCES conference_meetings (id) ON DELETE CASCADE,
    name       TEXT NOT NULL,
    email      TEXT,
    created_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS webinar_registrants_meeting
    ON webinar_registrants (tenant_id, meeting_id, created_at, id);
