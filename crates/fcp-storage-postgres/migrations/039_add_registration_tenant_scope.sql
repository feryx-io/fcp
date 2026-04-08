-- Add tenant scope and outbound flow-token support to SIP registrations.
-- Existing rows are backfilled into the nil/default tenant for compatibility.

ALTER TABLE registrations
    ADD COLUMN IF NOT EXISTS tenant_id TEXT;

UPDATE registrations
SET tenant_id = 'tenant-00000000-0000-0000-0000-000000000000'
WHERE tenant_id IS NULL;

ALTER TABLE registrations
    ALTER COLUMN tenant_id SET NOT NULL;

ALTER TABLE registrations
    ADD COLUMN IF NOT EXISTS flow_token TEXT;

ALTER TABLE registrations
    DROP CONSTRAINT IF EXISTS registrations_pkey;

ALTER TABLE registrations
    ADD CONSTRAINT registrations_pkey PRIMARY KEY (tenant_id, aor, contact);

CREATE INDEX IF NOT EXISTS idx_registrations_tenant_aor
    ON registrations(tenant_id, aor);
