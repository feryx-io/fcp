-- Provisioning that phones can use (docs/PROVISIONING.md,
-- docs/IDENTITY_AND_DEVICES.md phase 7a).

-- How a tenant's phones fetch: device credentials required, networks.
ALTER TABLE tenants ADD COLUMN IF NOT EXISTS provisioning_json JSONB NOT NULL DEFAULT '{}'::jsonb;

-- A tenant's key, in the path of its provisioning URL: found by its hash,
-- kept sealed so its administrator can be shown the URL again. A site's
-- phones fetch from the site, which makes its own.
CREATE TABLE IF NOT EXISTS provisioning_keys (
    -- The tenant's id (the re-encryption sweep finds rows by `id`).
    id TEXT PRIMARY KEY,
    key_hash TEXT NOT NULL UNIQUE,
    key_sealed TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Every fetch, the newest few hundred per tenant.
CREATE TABLE IF NOT EXISTS device_fetches (
    id BIGSERIAL PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    device_id TEXT,
    mac_address TEXT,
    file TEXT NOT NULL,
    at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    address TEXT,
    user_agent TEXT,
    outcome TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_device_fetches_device ON device_fetches (tenant_id, device_id, at DESC);
CREATE INDEX IF NOT EXISTS idx_device_fetches_tenant ON device_fetches (tenant_id, at DESC);
