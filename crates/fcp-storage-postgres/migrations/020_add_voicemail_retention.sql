-- Add message retention period to voicemail mailboxes.
-- NULL = keep forever (default, backward-compatible).
-- A value like 30 = new/heard messages expire 30 days after created_at.
ALTER TABLE voicemail_mailboxes
  ADD COLUMN IF NOT EXISTS retention_days INTEGER;
