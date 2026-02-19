-- Resource lists for RFC 4826 Resource List Subscriptions (RLS)
CREATE TABLE IF NOT EXISTS resource_lists (
    uri TEXT PRIMARY KEY,
    name TEXT,
    entries_json TEXT NOT NULL DEFAULT '[]',
    tenant_id TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_rls_tenant ON resource_lists(tenant_id);
