-- A deleted user is deprovisioned, then purged after the tenant's grace
-- period (docs/USERS.md §4, docs/IDENTITY_AND_DEVICES.md phase 5b).

-- When the user was deprovisioned; the sweep finds the due ones by it.
ALTER TABLE users ADD COLUMN IF NOT EXISTS deprovisioned_at TIMESTAMPTZ;
CREATE INDEX IF NOT EXISTS idx_users_deprovisioned
    ON users (tenant_id, deprovisioned_at)
    WHERE account_status = 'deprovisioned';

-- The tenant's grace period and the owner purged users' meetings move to.
ALTER TABLE tenants ADD COLUMN IF NOT EXISTS offboarding_json JSONB NOT NULL DEFAULT '{}'::jsonb;

-- A mailbox outlives its line: a purge releases the user's lines and keeps
-- the mailbox until voicemail retention empties it, and a line deleted on
-- its own no longer takes the user's messages with it. A site applies the
-- line's delete before the user's, so the cascade had to go everywhere.
-- Only a purge or a line's delete leaves the column empty; a node of the
-- previous release reading such a mailbox fails that one read.
ALTER TABLE voicemail_mailboxes ALTER COLUMN line_binding_id DROP NOT NULL;
ALTER TABLE voicemail_mailboxes DROP CONSTRAINT IF EXISTS voicemail_mailboxes_line_binding_id_fkey;
ALTER TABLE voicemail_mailboxes ADD CONSTRAINT voicemail_mailboxes_line_binding_id_fkey
    FOREIGN KEY (line_binding_id) REFERENCES line_bindings(id) ON DELETE SET NULL;
