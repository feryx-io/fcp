-- Zero-touch: vendors' redirection services (docs/PROVISIONING.md §10,
-- docs/IDENTITY_AND_DEVICES.md phase 8).

-- A tenant's account with a vendor's service: the secret sealed, the row
-- addressed by `id` (`<tenant>:<service>`) for the re-encryption sweep.
CREATE TABLE IF NOT EXISTS redirection_accounts (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    service TEXT NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT true,
    key_id TEXT NOT NULL,
    secret_sealed TEXT NOT NULL,
    account_id TEXT,
    profile TEXT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_redirection_accounts_tenant ON redirection_accounts (tenant_id);

-- Where each device stands with its vendor's service.
CREATE TABLE IF NOT EXISTS device_redirections (
    device_id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    service TEXT NOT NULL,
    mac_address TEXT NOT NULL,
    vendor_ref TEXT,
    state TEXT NOT NULL,
    detail TEXT,
    with_credentials BOOLEAN NOT NULL DEFAULT false,
    at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_device_redirections_tenant ON device_redirections (tenant_id);
