-- Webinars W6 (docs/WEBINARS.md §2.8): a webinar streamed out as HLS
-- while it is live. `hls` streams it; `off` or NULL does not.
ALTER TABLE conference_meetings
    ADD COLUMN IF NOT EXISTS streaming TEXT;
