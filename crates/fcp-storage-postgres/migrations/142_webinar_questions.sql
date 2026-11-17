-- Webinars W3.2 (docs/WEBINARS.md §2.5): questions from the audience.
--
-- A meeting's `qa` is `off`, `moderated` or `open`; NULL is the kind's
-- default (`moderated` for a webinar, `off` for a meeting).
-- `qa_anonymous` lets a question be asked without the asker's name.
ALTER TABLE conference_meetings
    ADD COLUMN IF NOT EXISTS qa TEXT,
    ADD COLUMN IF NOT EXISTS qa_anonymous BOOLEAN NOT NULL DEFAULT false;

-- Each question asked in a room, kept as the room holds it. `session_id`
-- is one sitting of the room (from its first question to its end);
-- `participant_id` is the asker's id in that sitting; `asker_name` is
-- NULL when the question was asked anonymously.
CREATE TABLE IF NOT EXISTS webinar_questions (
    id             TEXT PRIMARY KEY,
    tenant_id      TEXT NOT NULL,
    meeting_id     TEXT,
    room_number    TEXT NOT NULL,
    session_id     TEXT NOT NULL,
    participant_id TEXT NOT NULL,
    asker_name     TEXT,
    text           TEXT NOT NULL,
    status         TEXT NOT NULL DEFAULT 'pending',
    published      BOOLEAN NOT NULL DEFAULT false,
    upvotes        INTEGER NOT NULL DEFAULT 0,
    answer         TEXT,
    answered_live  BOOLEAN NOT NULL DEFAULT false,
    asked_at       TIMESTAMPTZ NOT NULL,
    updated_at     TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS webinar_questions_room
    ON webinar_questions (tenant_id, room_number, asked_at);
CREATE INDEX IF NOT EXISTS webinar_questions_session
    ON webinar_questions (session_id, asked_at);
