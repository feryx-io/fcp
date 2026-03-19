-- Add per-meeting sound file paths and TTS text fields to conference_meetings.
-- These allow individual meetings to override the global conference server
-- sound configuration with custom audio files or TTS-generated prompts.

ALTER TABLE conference_meetings
    ADD COLUMN IF NOT EXISTS pin_prompt_sound TEXT,
    ADD COLUMN IF NOT EXISTS invalid_pin_sound TEXT,
    ADD COLUMN IF NOT EXISTS moh_sound TEXT,
    ADD COLUMN IF NOT EXISTS pin_prompt_tts TEXT,
    ADD COLUMN IF NOT EXISTS invalid_pin_tts TEXT;
