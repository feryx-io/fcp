-- Add is_active flag to voicemail greetings
ALTER TABLE voicemail_greetings ADD COLUMN IF NOT EXISTS is_active BOOLEAN NOT NULL DEFAULT TRUE;
