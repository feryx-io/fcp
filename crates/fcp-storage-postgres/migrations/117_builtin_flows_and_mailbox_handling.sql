-- Built-in flows are no longer stored (docs/DEEP_TESTING.md D8): a call's
-- inline flow (`vm-access-<call>`, `hoteling-login-<call>`) was written at
-- every `*98` and hoteling login and never removed, and each mailbox's
-- `vm-auto-<mailbox>` flow belonged to no tenant, so no one could see or
-- edit it. The nodes hold them in memory now; the rows go (their versions
-- with them, by the foreign key).
DELETE FROM flows
 WHERE id LIKE 'vm-access-%'
    OR id LIKE 'vm-auto-%'
    OR id LIKE 'hoteling-login-%';

-- A mailbox's call handling: a flow of its tenant's that the call manager
-- runs where the mailbox would have taken the call — say something, ring
-- someone else, a queue, or a message after all. NULL: the mailbox itself.
ALTER TABLE voicemail_mailboxes ADD COLUMN IF NOT EXISTS flow_id VARCHAR(255);
