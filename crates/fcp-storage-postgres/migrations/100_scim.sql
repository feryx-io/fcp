-- SCIM 2.0 provisioning (docs/SCIM.md, docs/IDENTITY_AND_DEVICES.md phase 6):
-- which users the tenant's provisioner made, and its own id for each.
ALTER TABLE users ADD COLUMN IF NOT EXISTS scim_managed BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE users ADD COLUMN IF NOT EXISTS scim_external_id TEXT;
CREATE UNIQUE INDEX IF NOT EXISTS idx_users_tenant_scim_external_id
    ON users (tenant_id, scim_external_id)
    WHERE scim_external_id IS NOT NULL;
