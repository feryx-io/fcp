-- Tenant trunks phase 6c (docs/TENANT_TRUNKS.md §12): a tenant's own
-- least-cost routes. Each prices a dialled-number prefix on one of the
-- tenant's trunks; a node orders that tenant's trunks by them, longest
-- prefix first, then cost, then priority. A route goes with its trunk.

CREATE TABLE IF NOT EXISTS lcr_routes (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    prefix TEXT NOT NULL,
    trunk_id TEXT NOT NULL REFERENCES trunks (id) ON DELETE CASCADE,
    cost_per_minute NUMERIC(18, 6) NOT NULL,
    priority INTEGER NOT NULL DEFAULT 100,
    enabled BOOLEAN NOT NULL DEFAULT true,
    description TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, prefix, trunk_id)
);

CREATE INDEX IF NOT EXISTS idx_lcr_routes_tenant ON lcr_routes (tenant_id, prefix);
