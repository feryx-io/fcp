ALTER TABLE conference_meetings ADD COLUMN IF NOT EXISTS owner_id TEXT;
CREATE INDEX IF NOT EXISTS idx_conference_meetings_owner ON conference_meetings(tenant_id, owner_id);
