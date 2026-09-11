-- A shared screen (docs/VIDEO_CONFERENCING.md §15.6): who may share one,
-- and how the composite shows it.
-- video_content_share: 'off' (no content section is answered), 'hosts'
-- (a host's is; a guest's is answered but never given the floor) or
-- 'anyone'.
-- video_content_layout: 'presentation' (the screen large, cameras in a
-- strip) or 'content_only' (the screen alone).
ALTER TABLE conference_meetings
    ADD COLUMN IF NOT EXISTS video_content_share TEXT NOT NULL DEFAULT 'anyone',
    ADD COLUMN IF NOT EXISTS video_content_layout TEXT NOT NULL DEFAULT 'presentation';
