-- When a participant whose view is a single source they can already decode
-- is sent that source's own stream rather than a composite
-- (docs/VIDEO_CONFERENCING.md §5.6): 'off', 'auto' or 'on'.
-- 'auto' forwards only while the room shows one participant full canvas,
-- where the missing name banner is least missed.
ALTER TABLE conference_meetings
    ADD COLUMN IF NOT EXISTS video_passthrough TEXT NOT NULL DEFAULT 'auto';
