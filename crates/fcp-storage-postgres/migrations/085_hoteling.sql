-- Enterprise calling, phase 8 (docs/ENTERPRISE_CALLING.md §3.8): hoteling.
-- A device may have credentials of its own, so a shared phone registers
-- as itself rather than as whoever owns it; a host is a phone a user may
-- log in on, and a session is a guest on a host.

-- The device's own digest credentials. `sip_ha1_hash` and
-- `sip_provisioning_password` are sealed at rest like a user's.
ALTER TABLE devices ADD COLUMN IF NOT EXISTS sip_username TEXT;
ALTER TABLE devices ADD COLUMN IF NOT EXISTS sip_ha1_hash TEXT;
ALTER TABLE devices ADD COLUMN IF NOT EXISTS sip_provisioning_password TEXT;
ALTER TABLE devices ADD COLUMN IF NOT EXISTS hoteling_host BOOLEAN NOT NULL DEFAULT FALSE;

CREATE UNIQUE INDEX IF NOT EXISTS idx_devices_tenant_sip_username
    ON devices (tenant_id, LOWER(sip_username))
    WHERE sip_username IS NOT NULL;

-- Sessions are where a login happened: a site's are its own, so the
-- table is not journaled (docs/HYBRID_CONTROL_PLANE.md §3.2).
CREATE TABLE IF NOT EXISTS hoteling_sessions (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    device_id TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
    user_id TEXT NOT NULL,
    started_at TIMESTAMPTZ NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    ended_at TIMESTAMPTZ,
    ended_by TEXT
);

CREATE INDEX IF NOT EXISTS idx_hoteling_sessions_tenant
    ON hoteling_sessions (tenant_id, started_at DESC);

-- One guest per host, one host per guest.
CREATE UNIQUE INDEX IF NOT EXISTS idx_hoteling_sessions_active_device
    ON hoteling_sessions (device_id) WHERE ended_at IS NULL;
CREATE UNIQUE INDEX IF NOT EXISTS idx_hoteling_sessions_active_user
    ON hoteling_sessions (user_id) WHERE ended_at IS NULL;

-- The sweep's reach.
CREATE INDEX IF NOT EXISTS idx_hoteling_sessions_open_expiry
    ON hoteling_sessions (expires_at) WHERE ended_at IS NULL;
