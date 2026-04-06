-- SIP Trunks table
CREATE TABLE IF NOT EXISTS trunks (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    name TEXT NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT true,
    direction TEXT NOT NULL DEFAULT 'upstream',
    priority INTEGER NOT NULL DEFAULT 10,
    weight INTEGER NOT NULL DEFAULT 100,
    hosts TEXT[] NOT NULL DEFAULT '{}',
    transport_protocol TEXT NOT NULL DEFAULT 'udp',
    transport_port INTEGER NOT NULL DEFAULT 5060,
    verify_tls BOOLEAN NOT NULL DEFAULT true,
    auth_username TEXT,
    auth_password TEXT,
    auth_realm TEXT,
    register BOOLEAN NOT NULL DEFAULT false,
    register_expiry INTEGER NOT NULL DEFAULT 3600,
    max_concurrent_calls INTEGER NOT NULL DEFAULT 0,
    max_failures INTEGER NOT NULL DEFAULT 5,
    recovery_interval_secs INTEGER NOT NULL DEFAULT 60,
    health_check_enabled BOOLEAN NOT NULL DEFAULT true,
    health_check_interval_secs INTEGER NOT NULL DEFAULT 30,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(tenant_id, name)
);

CREATE INDEX IF NOT EXISTS idx_trunks_tenant ON trunks(tenant_id);
CREATE INDEX IF NOT EXISTS idx_trunks_name ON trunks(tenant_id, name);
