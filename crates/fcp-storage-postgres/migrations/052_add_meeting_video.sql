-- Per-meeting video settings (docs/VIDEO_CONFERENCING.md §12 "Meeting model").
-- video_codecs is a comma-separated list of SDP encoding names in preference order.
ALTER TABLE conference_meetings ADD COLUMN IF NOT EXISTS video_enabled BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE conference_meetings ADD COLUMN IF NOT EXISTS video_layout TEXT NOT NULL DEFAULT 'grid';
ALTER TABLE conference_meetings ADD COLUMN IF NOT EXISTS video_max_participants INTEGER NOT NULL DEFAULT 16;
ALTER TABLE conference_meetings ADD COLUMN IF NOT EXISTS video_max_resolution TEXT NOT NULL DEFAULT '720p';
ALTER TABLE conference_meetings ADD COLUMN IF NOT EXISTS video_fps INTEGER NOT NULL DEFAULT 15;
ALTER TABLE conference_meetings ADD COLUMN IF NOT EXISTS video_exclude_self BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE conference_meetings ADD COLUMN IF NOT EXISTS video_codecs TEXT NOT NULL DEFAULT 'H264,VP8';
