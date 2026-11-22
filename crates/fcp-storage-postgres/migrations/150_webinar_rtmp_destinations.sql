-- Webinars W7 (docs/WEBINARS.md §2.8): where a webinar is pushed over RTMP.
-- The stream key is sealed at rest (encryption.rs SEALED_COLUMNS) and the
-- table is apart from conference_meetings, so the key never travels with a
-- meeting to a site by the sync journal. `id` is the meeting's id, the name
-- the re-encrypt pass addresses a sealed row by.
CREATE TABLE IF NOT EXISTS webinar_rtmp_destinations (
    id TEXT PRIMARY KEY REFERENCES conference_meetings (id) ON DELETE CASCADE,
    tenant_id TEXT NOT NULL,
    url TEXT NOT NULL,
    key_sealed TEXT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_webinar_rtmp_destinations_tenant
    ON webinar_rtmp_destinations (tenant_id);
