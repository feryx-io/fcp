-- How a tenant's people sign in (docs/IDENTITY_AND_DEVICES.md phase 2):
-- the email domains that are the tenant's, so a sign-in finds its tenant
-- before it knows the user, and the tenant's sign-in policy.

ALTER TABLE tenants
    ADD COLUMN IF NOT EXISTS login_domains TEXT[] NOT NULL DEFAULT ARRAY[]::TEXT[];

-- `WHERE $1 = ANY(login_domains)`; a domain is one tenant's, which the
-- API checks before it writes.
CREATE INDEX IF NOT EXISTS idx_tenants_login_domains ON tenants USING GIN (login_domains);

-- `{"password_login": "allowed" | "admins_only" | "disabled"}`.
ALTER TABLE tenants
    ADD COLUMN IF NOT EXISTS sign_in_json JSONB NOT NULL DEFAULT '{}'::jsonb;
