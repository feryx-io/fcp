-- Lines on a device (docs/DEVICE_LINES.md): the accounts a device
-- registers and takes calls on, in order (its own, another extension its
-- owner holds, a shared line, a user its owner acts for); a tenant's
-- choice whether its users may name their own lines; and the names they
-- chose, by the line itself, so every device showing it shows the name.
ALTER TABLE devices
    ADD COLUMN IF NOT EXISTS lines_json JSONB NOT NULL DEFAULT '[]'::jsonb;
ALTER TABLE tenants
    ADD COLUMN IF NOT EXISTS lines_json JSONB NOT NULL DEFAULT '{}'::jsonb;

CREATE TABLE IF NOT EXISTS user_line_labels (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    tenant_id TEXT NOT NULL,
    -- `own`, `extension:1005`, `shared:+15551234567`, `on_behalf:<user>`.
    line TEXT NOT NULL,
    label TEXT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (user_id, line)
);
CREATE INDEX IF NOT EXISTS idx_user_line_labels_tenant ON user_line_labels (tenant_id);
