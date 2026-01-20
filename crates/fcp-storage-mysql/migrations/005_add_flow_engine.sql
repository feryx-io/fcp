-- Flow engine schema
-- Version: 0.8.0
-- This migration adds tables for flow definitions (programmable call flows)

-- Flows table
CREATE TABLE IF NOT EXISTS flows (
    id VARCHAR(255) PRIMARY KEY,
    name VARCHAR(255) NOT NULL,
    description TEXT,
    version INT NOT NULL DEFAULT 1,
    initial_node VARCHAR(255) NOT NULL,
    nodes JSON NOT NULL, -- JSON serialized HashMap<String, FlowNode>
    variables JSON NOT NULL, -- JSON serialized HashMap<String, String>
    tenant_id VARCHAR(255),
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    webhook_url TEXT,

    -- Audit
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP,

    -- Unique constraint for (id, version) to support versioning
    UNIQUE KEY unique_id_version (id, version)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE INDEX idx_flows_tenant ON flows(tenant_id);
CREATE INDEX idx_flows_enabled ON flows(enabled);
CREATE INDEX idx_flows_name ON flows(name);
CREATE INDEX idx_flows_updated_at ON flows(updated_at);

-- View to get latest version of each flow
CREATE OR REPLACE VIEW flows_latest AS
SELECT f.*
FROM flows f
INNER JOIN (
    SELECT id, MAX(version) as max_version
    FROM flows
    GROUP BY id
) latest ON f.id = latest.id AND f.version = latest.max_version;
