-- Add per-meeting welcome TTS text field.
-- Supports {count} placeholder for dynamic participant count.
ALTER TABLE conference_meetings
    ADD COLUMN IF NOT EXISTS welcome_tts TEXT;
