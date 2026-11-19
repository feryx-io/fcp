-- Webinars W3b (docs/WEBINARS.md §2.5): chat in every room.
--
-- A meeting's `chat` is `everyone`, `hosts` or `off`; NULL is the kind's
-- default (`everyone` for a meeting, `hosts` for a webinar).
ALTER TABLE conference_meetings
    ADD COLUMN IF NOT EXISTS chat TEXT;

-- Each message posted in a room. `session_id` is one sitting of the room
-- (from its first participant to its last); `participant_id` is the
-- sender's id in that sitting; `sender_address` is where they called from
-- (their From URI), which a subject request finds them by with
-- `sender_name`. `audience` is `room` or `hosts`. A deleted message keeps
-- its row, with when and by whom (a participant id, NULL through the API).
CREATE TABLE IF NOT EXISTS conference_chat (
    id             TEXT PRIMARY KEY,
    tenant_id      TEXT NOT NULL,
    meeting_id     TEXT,
    room_number    TEXT NOT NULL,
    session_id     TEXT NOT NULL,
    participant_id TEXT NOT NULL,
    sender_name    TEXT,
    sender_address TEXT,
    audience       TEXT NOT NULL DEFAULT 'room',
    text           TEXT NOT NULL,
    sent_at        TIMESTAMPTZ NOT NULL,
    deleted_at     TIMESTAMPTZ,
    deleted_by     TEXT
);

CREATE INDEX IF NOT EXISTS conference_chat_room
    ON conference_chat (tenant_id, room_number, sent_at);
CREATE INDEX IF NOT EXISTS conference_chat_session
    ON conference_chat (session_id, sent_at);
CREATE INDEX IF NOT EXISTS conference_chat_sent
    ON conference_chat (tenant_id, sent_at);
