-- Webinars (docs/WEBINARS.md §2.1): a meeting's kind, 'meeting' (anyone
-- may speak) or 'webinar' (a stage heard by an audience); a webinar's
-- panelist PIN, sealed at rest as the other PINs are; and the users, by
-- id, who are its panelists.
ALTER TABLE conference_meetings
    ADD COLUMN IF NOT EXISTS kind TEXT NOT NULL DEFAULT 'meeting',
    ADD COLUMN IF NOT EXISTS panelist_pin TEXT,
    ADD COLUMN IF NOT EXISTS panelists TEXT[] NOT NULL DEFAULT '{}';
