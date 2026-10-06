-- A tenant's brand (docs/BRANDING.md, docs/SOFTPHONE.md phase F3): what
-- its softphone takes on, served at /brand/v1.

-- Uploaded files, by their SHA-256: logos, the icon, ringtones. Small (at
-- most 5 MB), so kept here, where every admin and every backup sees them.
CREATE TABLE IF NOT EXISTS brand_assets (
    tenant_id TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    sha256 TEXT NOT NULL,
    mime TEXT NOT NULL,
    size BIGINT NOT NULL,
    width INTEGER,
    height INTEGER,
    file_name TEXT,
    bytes BYTEA NOT NULL,
    uploaded_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, sha256)
);
-- Served by hash, whoever uploaded it.
CREATE INDEX IF NOT EXISTS idx_brand_assets_sha256 ON brand_assets (sha256);

-- The brand itself, one per tenant, naming its assets by hash.
CREATE TABLE IF NOT EXISTS tenant_brands (
    tenant_id TEXT PRIMARY KEY REFERENCES tenants(id) ON DELETE CASCADE,
    brand JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Synced to the tenant's sites like the rest of its configuration; the
-- link fetches the assets the brand names by hash.
DROP TRIGGER IF EXISTS sync_journal ON tenant_brands;
CREATE TRIGGER sync_journal AFTER INSERT OR UPDATE OR DELETE ON tenant_brands
    FOR EACH ROW EXECUTE FUNCTION fcp_sync_journal('tenant_id', 'tenant_id');
