-- Flow engine schema
-- Version: 0.8.0
-- This migration adds tables for flow definitions (programmable call flows)

-- Flows table
CREATE TABLE IF NOT EXISTS flows (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    description TEXT,
    version INTEGER NOT NULL DEFAULT 1,
    initial_node TEXT NOT NULL,
    nodes TEXT NOT NULL, -- JSON serialized HashMap<String, FlowNode>
    variables TEXT NOT NULL, -- JSON serialized HashMap<String, String>
    tenant_id TEXT,
    enabled INTEGER NOT NULL DEFAULT 1,
    webhook_url TEXT,

    -- Audit
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,

    -- Unique constraint for (id, version) to support versioning
    UNIQUE(id, version)
);

CREATE INDEX IF NOT EXISTS idx_flows_tenant ON flows(tenant_id);
CREATE INDEX IF NOT EXISTS idx_flows_enabled ON flows(enabled);
CREATE INDEX IF NOT EXISTS idx_flows_name ON flows(name);
CREATE INDEX IF NOT EXISTS idx_flows_updated_at ON flows(updated_at);

-- View to get latest version of each flow
CREATE VIEW IF NOT EXISTS flows_latest AS
SELECT f.*
FROM flows f
INNER JOIN (
    SELECT id, MAX(version) as max_version
    FROM flows
    GROUP BY id
) latest ON f.id = latest.id AND f.version = latest.max_version;
