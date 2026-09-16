-- E911 block 2 phase 3 (docs/E911_BLOCK_2.md §6): where the device is.
--
-- A tenant maps its networks to sites, so a device that moves is located by
-- the address it is seen at rather than where it was provisioned; a user who
-- is on no known network says where they are, and that stands for a while.
-- Registrations keep the address they came from, devices may be trusted to
-- send their own location, and the tenant says which device types are.

CREATE TABLE IF NOT EXISTS emergency_networks (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    cidr TEXT NOT NULL,
    location_id TEXT NOT NULL,
    name TEXT,
    priority INTEGER NOT NULL DEFAULT 0,
    -- 'nomadic' (softphones, mobile, browsers) or 'all'.
    applies_to TEXT NOT NULL DEFAULT 'nomadic',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_emergency_networks_tenant
    ON emergency_networks (tenant_id);

-- Where a user last said they were. One current declaration per user.
CREATE TABLE IF NOT EXISTS declared_locations (
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    address JSONB NOT NULL,
    declared_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (tenant_id, user_id)
);

ALTER TABLE registrations ADD COLUMN IF NOT EXISTS source_address TEXT;
ALTER TABLE devices ADD COLUMN IF NOT EXISTS e911_trust_location BOOLEAN;

ALTER TABLE emergency_configs
    ADD COLUMN IF NOT EXISTS trust_device_locations TEXT[] NOT NULL DEFAULT ARRAY[]::TEXT[],
    ADD COLUMN IF NOT EXISTS declared_location_ttl_secs BIGINT NOT NULL DEFAULT 86400;
