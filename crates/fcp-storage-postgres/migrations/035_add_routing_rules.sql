-- Routing rules table for call routing policy and trunk selection
CREATE TABLE IF NOT EXISTS routing_rules (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    name TEXT NOT NULL,
    description TEXT,
    priority INTEGER NOT NULL DEFAULT 100,
    enabled BOOLEAN NOT NULL DEFAULT true,

    -- Match conditions
    destination_pattern TEXT,           -- regex for dialed number
    source_pattern TEXT,                -- regex for caller number/URI
    source_ip_ranges TEXT[],            -- CIDR ranges for source IP matching
    time_window_days TEXT[],            -- e.g. {"Monday","Tuesday",...}
    time_window_start TEXT,             -- HH:MM (24h)
    time_window_end TEXT,               -- HH:MM (24h)
    time_window_timezone TEXT DEFAULT 'UTC',

    -- Action
    action TEXT NOT NULL DEFAULT 'allow',  -- allow, deny, route_to_trunk
    target_trunk_id TEXT,               -- FK to trunks.id (for route_to_trunk)
    target_trunk_group TEXT,            -- match trunks by group name (failover)

    -- Metadata
    reason TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    UNIQUE(tenant_id, name)
);

CREATE INDEX IF NOT EXISTS idx_routing_rules_tenant ON routing_rules(tenant_id);
CREATE INDEX IF NOT EXISTS idx_routing_rules_priority ON routing_rules(tenant_id, priority);
