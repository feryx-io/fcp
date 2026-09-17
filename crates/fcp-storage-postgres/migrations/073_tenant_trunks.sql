-- Tenant trunks phase 2 (docs/TENANT_TRUNKS.md §5): a tenant's own trunks
-- carry its calls.
--
-- A tenant whose trunks cannot take a call falls back to the platform's —
-- the trunks each node reads from its own [[trunks]] — unless it says
-- otherwise. Every tenant that existed before this behaves as it did.

ALTER TABLE tenants
    ADD COLUMN IF NOT EXISTS platform_trunk_fallback BOOLEAN NOT NULL DEFAULT TRUE;

-- The call's record says which trunk carried it, not just what it was
-- called: two tenants may both have a "carrier-a".
ALTER TABLE cdrs
    ADD COLUMN IF NOT EXISTS trunk_id TEXT;

CREATE INDEX IF NOT EXISTS idx_cdrs_trunk_id
    ON cdrs (tenant_id, trunk_id, start_time DESC)
    WHERE trunk_id IS NOT NULL;
