ALTER TABLE conference_meetings ADD COLUMN IF NOT EXISTS play_entry_exit_tones BOOLEAN NOT NULL DEFAULT TRUE;
ALTER TABLE conference_meetings ADD COLUMN IF NOT EXISTS join_sound TEXT;
ALTER TABLE conference_meetings ADD COLUMN IF NOT EXISTS exit_sound TEXT;
