-- Locations table for multi-location support
CREATE TABLE IF NOT EXISTS locations (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    name TEXT NOT NULL,
    description TEXT,
    address_line1 TEXT,
    address_line2 TEXT,
    city TEXT,
    state TEXT,
    postal_code TEXT,
    country TEXT,
    floor TEXT,
    building TEXT,
    latitude DOUBLE PRECISION,
    longitude DOUBLE PRECISION,
    phone_number TEXT,
    timezone TEXT,
    emergency_contact_name TEXT,
    emergency_contact_phone TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_locations_tenant ON locations(tenant_id);
CREATE INDEX IF NOT EXISTS idx_locations_name ON locations(tenant_id, name);

-- Add location_id to users, devices, extensions
ALTER TABLE users ADD COLUMN IF NOT EXISTS location_id TEXT;
ALTER TABLE devices ADD COLUMN IF NOT EXISTS location_id TEXT;
ALTER TABLE extensions ADD COLUMN IF NOT EXISTS location_id TEXT;
