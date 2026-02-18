-- Migration 014: Add presence presentity state persistence
CREATE TABLE IF NOT EXISTS presence_presentities (
    aor TEXT PRIMARY KEY,
    state TEXT NOT NULL,
    note TEXT,
    tenant_id TEXT NOT NULL,
    updated_at_epoch INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_presence_presentity_tenant ON presence_presentities(tenant_id);
