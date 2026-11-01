-- A routing rule may be for one location's callers (docs/TENANT_SCOPE.md
-- §3.6). NULL: every call of the tenant's, as every rule was before.
ALTER TABLE routing_rules ADD COLUMN IF NOT EXISTS location_id TEXT;

CREATE INDEX IF NOT EXISTS idx_routing_rules_location
    ON routing_rules (tenant_id, location_id)
    WHERE location_id IS NOT NULL;
