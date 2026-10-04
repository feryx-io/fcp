-- Provisioning: templates by model, line keys, pending phones
-- (docs/PROVISIONING.md, docs/IDENTITY_AND_DEVICES.md phase 7b).

-- A phone's vendor, which with its model picks its template, and the
-- keys programmed on it.
ALTER TABLE devices ADD COLUMN IF NOT EXISTS vendor TEXT;
ALTER TABLE devices ADD COLUMN IF NOT EXISTS line_keys_json JSONB NOT NULL DEFAULT '[]'::jsonb;

-- Phones that fetched under a tenant's key with a MAC no device has, to
-- be claimed. A site's are its own.
CREATE TABLE IF NOT EXISTS pending_devices (
    tenant_id TEXT NOT NULL,
    mac_address TEXT NOT NULL,
    first_seen TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_seen TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    address TEXT,
    user_agent TEXT,
    vendor TEXT,
    model TEXT,
    firmware TEXT,
    fetches INTEGER NOT NULL DEFAULT 1,
    PRIMARY KEY (tenant_id, mac_address)
);
CREATE INDEX IF NOT EXISTS idx_pending_devices_seen ON pending_devices (tenant_id, last_seen DESC);
