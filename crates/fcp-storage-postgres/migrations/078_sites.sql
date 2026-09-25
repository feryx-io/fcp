-- Sites: a tenant's own stack on its own network, enrolled to this cloud
-- and kept by fcp-link (docs/HYBRID_CONTROL_PLANE.md phase 1). The row is
-- the site as the cloud keeps it (site_json), with the columns the cloud
-- looks it up by. The enrollment token is kept as a hash and spent once.
CREATE TABLE IF NOT EXISTS sites (
    id            TEXT PRIMARY KEY,
    tenant_id     TEXT NOT NULL,
    name          TEXT NOT NULL,
    status        TEXT NOT NULL,
    secret_hash   TEXT,
    created_at    TIMESTAMPTZ NOT NULL,
    updated_at    TIMESTAMPTZ NOT NULL,
    site_json     JSONB NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_sites_tenant ON sites (tenant_id, created_at DESC);
CREATE UNIQUE INDEX IF NOT EXISTS idx_sites_secret_hash ON sites (secret_hash)
    WHERE secret_hash IS NOT NULL;

CREATE TABLE IF NOT EXISTS site_enrollment_tokens (
    token_hash    TEXT PRIMARY KEY,
    site_id       TEXT NOT NULL REFERENCES sites (id) ON DELETE CASCADE,
    created_at    TIMESTAMPTZ NOT NULL,
    expires_at    TIMESTAMPTZ NOT NULL,
    used_at       TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS idx_site_enrollment_tokens_site ON site_enrollment_tokens (site_id);
