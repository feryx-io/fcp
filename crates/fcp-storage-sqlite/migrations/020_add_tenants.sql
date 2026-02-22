-- Tenant management table
CREATE TABLE IF NOT EXISTS tenants (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL UNIQUE,
    description TEXT,
    status TEXT NOT NULL DEFAULT 'active',
    contact_email TEXT,
    contact_phone TEXT,
    max_users INTEGER,
    max_phone_numbers INTEGER,
    max_concurrent_calls INTEGER,
    billing_id TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    trial_expires_at INTEGER,
    metadata TEXT
);

CREATE INDEX IF NOT EXISTS idx_tenants_name ON tenants(name);
CREATE INDEX IF NOT EXISTS idx_tenants_status ON tenants(status);
