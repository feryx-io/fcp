-- Flow engine schema
-- Version: 0.8.0
-- This migration adds tables for flow definitions (programmable call flows)

-- Flows table
CREATE TABLE IF NOT EXISTS flows (
    id VARCHAR(255) PRIMARY KEY,
    name VARCHAR(255) NOT NULL,
    description TEXT,
    version INTEGER NOT NULL DEFAULT 1,
    initial_node VARCHAR(255) NOT NULL,
    nodes JSONB NOT NULL, -- JSON serialized HashMap<String, FlowNode>
    variables JSONB NOT NULL, -- JSON serialized HashMap<String, String>
    tenant_id VARCHAR(255),
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    webhook_url TEXT,

    -- Audit
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Unique constraint for (id, version) to support versioning
    UNIQUE(id, version)
);

CREATE INDEX IF NOT EXISTS idx_flows_tenant ON flows(tenant_id);
CREATE INDEX IF NOT EXISTS idx_flows_enabled ON flows(enabled);
CREATE INDEX IF NOT EXISTS idx_flows_name ON flows(name);
CREATE INDEX IF NOT EXISTS idx_flows_updated_at ON flows(updated_at);

-- GIN indexes for JSONB columns to enable efficient querying
CREATE INDEX IF NOT EXISTS idx_flows_nodes ON flows USING GIN (nodes);
CREATE INDEX IF NOT EXISTS idx_flows_variables ON flows USING GIN (variables);

-- View to get latest version of each flow
CREATE VIEW IF NOT EXISTS flows_latest AS
SELECT f.*
FROM flows f
INNER JOIN (
    SELECT id, MAX(version) as max_version
    FROM flows
    GROUP BY id
) latest ON f.id = latest.id AND f.version = latest.max_version;

-- Trigger to update updated_at timestamp
CREATE OR REPLACE FUNCTION update_flows_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER flows_updated_at_trigger
    BEFORE UPDATE ON flows
    FOR EACH ROW
    EXECUTE FUNCTION update_flows_updated_at();
