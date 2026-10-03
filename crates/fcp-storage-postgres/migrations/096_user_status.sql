-- A user's lifecycle is one status (docs/IDENTITY_AND_DEVICES.md phase 5):
-- `enabled` is read from it and kept for one release. Rows from before
-- agree with themselves from here: a disabled user is suspended, and a
-- suspended one is not enabled.
UPDATE users SET account_status = 'suspended'
    WHERE NOT enabled AND account_status <> 'suspended';
UPDATE users SET enabled = (account_status <> 'suspended')
    WHERE enabled <> (account_status <> 'suspended');

-- A tenant's users are listed and counted in SQL.
CREATE INDEX IF NOT EXISTS idx_users_tenant_username ON users (tenant_id, username);
