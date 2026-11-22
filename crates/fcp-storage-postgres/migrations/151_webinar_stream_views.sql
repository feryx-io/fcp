-- Webinars W8 (docs/WEBINARS.md): who watched a sitting's HLS stream by a
-- registrant's link, and how long. A row goes with its registration, so
-- retention, legal holds and subject erasure of registrations cover it.
CREATE TABLE IF NOT EXISTS webinar_stream_views (
    sitting_id TEXT NOT NULL,
    registrant_id TEXT NOT NULL REFERENCES webinar_registrants (id) ON DELETE CASCADE,
    tenant_id TEXT NOT NULL,
    meeting_id TEXT NOT NULL,
    first_at TIMESTAMPTZ NOT NULL,
    last_at TIMESTAMPTZ NOT NULL,
    watched_secs BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (sitting_id, registrant_id)
);

CREATE INDEX IF NOT EXISTS idx_webinar_stream_views_registrant
    ON webinar_stream_views (registrant_id);
