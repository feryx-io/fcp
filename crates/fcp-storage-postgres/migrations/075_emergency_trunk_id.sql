-- Tenant trunks phase 6 (docs/TENANT_TRUNKS.md §5): a tenant's own
-- emergency-capable trunk carries its 911 calls, and a trunk's name is
-- unique only within its tenant, so an emergency call's record names the
-- trunk by id beside the name, as a CDR has since migration 073.

ALTER TABLE emergency_calls
    ADD COLUMN IF NOT EXISTS trunk_id TEXT;
