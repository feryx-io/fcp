-- SCIM groups (docs/SCIM.md, docs/IDENTITY_AND_DEVICES.md phase 6b): the
-- provisioner's own id for each group.
ALTER TABLE groups ADD COLUMN IF NOT EXISTS scim_external_id TEXT;
CREATE UNIQUE INDEX IF NOT EXISTS idx_groups_tenant_scim_external_id
    ON groups (tenant_id, scim_external_id)
    WHERE scim_external_id IS NOT NULL;
