-- A user's SIP name, kept across a rename (docs/USERS.md §6,
-- docs/IDENTITY_AND_DEVICES.md phase 5d): the SIP digest (HA1) is computed
-- over it, so a renamed user's phones keep registering as they did. NULL
-- is the username.
ALTER TABLE users ADD COLUMN IF NOT EXISTS sip_username TEXT;

-- One phone identity per name in a tenant: what SIP lookups find a user by.
CREATE UNIQUE INDEX IF NOT EXISTS idx_users_tenant_sip_name
    ON users (tenant_id, (COALESCE(sip_username, username)));
