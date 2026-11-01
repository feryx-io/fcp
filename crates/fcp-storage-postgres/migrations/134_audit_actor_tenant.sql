-- An audit row belongs to the tenant acted on (docs/TENANT_SCOPE.md §3.3):
-- `tenant_id` is that tenant, or NULL for a platform-level action (the
-- cluster, services, encryption, firmware images, tenant creation's
-- platform side). Where the actor came from is kept apart: the actor's own
-- tenant, NULL for the static token. Rows written before this release keep
-- their tenant (the actor's) and have no actor tenant.
ALTER TABLE admin_audit_log ADD COLUMN IF NOT EXISTS actor_tenant_id TEXT;

-- The platform's own rows, listed by a platform administrator.
CREATE INDEX IF NOT EXISTS idx_admin_audit_platform_time
    ON admin_audit_log (occurred_at DESC) WHERE tenant_id IS NULL;
