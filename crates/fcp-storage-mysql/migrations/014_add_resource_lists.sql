-- Resource lists for RFC 4826 Resource List Subscriptions (RLS)
CREATE TABLE IF NOT EXISTS resource_lists (
    uri VARCHAR(512) PRIMARY KEY,
    name VARCHAR(255),
    entries_json TEXT NOT NULL,
    tenant_id VARCHAR(255) NOT NULL,
    INDEX idx_rls_tenant (tenant_id)
);
