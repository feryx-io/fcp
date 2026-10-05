-- Managed firmware (docs/PROVISIONING.md §11, docs/IDENTITY_AND_DEVICES.md
-- phase 9).

-- Images the platform administrator uploads, every tenant's to use; the
-- file is kept in the admin's firmware directory under the id.
CREATE TABLE IF NOT EXISTS firmware_images (
    id TEXT PRIMARY KEY,
    vendor TEXT NOT NULL,
    models JSONB NOT NULL DEFAULT '[]'::jsonb,
    version TEXT NOT NULL,
    file_name TEXT NOT NULL,
    size BIGINT NOT NULL,
    sha256 TEXT NOT NULL,
    notes TEXT,
    uploaded_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- A tenant's choice of image for a vendor's models, rolled out to a pilot
-- set first or to every phone.
CREATE TABLE IF NOT EXISTS firmware_targets (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    vendor TEXT NOT NULL,
    model_pattern TEXT,
    image_id TEXT NOT NULL REFERENCES firmware_images(id),
    stage TEXT NOT NULL,
    pilot_devices JSONB NOT NULL DEFAULT '[]'::jsonb,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_firmware_targets_tenant ON firmware_targets (tenant_id);
CREATE INDEX IF NOT EXISTS idx_firmware_targets_image ON firmware_targets (image_id);
