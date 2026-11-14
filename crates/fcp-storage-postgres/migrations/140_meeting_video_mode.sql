-- Who is sent the room's composite (docs/FORWARDING_ROOMS.md §2.1):
-- 'mix', everyone; 'auto', everyone but a client that asks for each
-- person's own stream (Recv-Info: fcp-view).
ALTER TABLE conference_meetings
    ADD COLUMN IF NOT EXISTS video_mode TEXT NOT NULL DEFAULT 'mix';
