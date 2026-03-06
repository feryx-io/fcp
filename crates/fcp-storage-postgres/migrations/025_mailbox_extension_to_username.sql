-- Migrate voicemail mailbox extension field to use the owner's username.
-- The extension field now stores the SIP username rather than a separate
-- extension number, since voicemail routing resolves by username + aliases.

UPDATE voicemail_mailboxes
SET extension = users.username,
    updated_at = NOW()
FROM users
WHERE voicemail_mailboxes.user_id = users.id
  AND voicemail_mailboxes.extension != users.username;

-- Drop the old unique constraint on (tenant_id, extension) and replace
-- with (tenant_id, user_id) since mailboxes are now per-user.
ALTER TABLE voicemail_mailboxes DROP CONSTRAINT IF EXISTS voicemail_mailboxes_tenant_id_extension_key;
ALTER TABLE voicemail_mailboxes ADD CONSTRAINT voicemail_mailboxes_tenant_id_user_id_key UNIQUE (tenant_id, user_id);
