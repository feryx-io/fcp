-- A voicemail message's file size, written when the message is saved, so a
-- mailbox's storage is summed rather than reported as zero. Messages saved
-- before this migration have none and count as nothing.
ALTER TABLE voicemail_messages ADD COLUMN IF NOT EXISTS size_bytes BIGINT;
