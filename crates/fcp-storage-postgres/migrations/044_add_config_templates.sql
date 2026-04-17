CREATE TABLE IF NOT EXISTS config_templates (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    name TEXT NOT NULL,
    vendor TEXT NOT NULL,
    model_pattern TEXT,
    template_body TEXT NOT NULL,
    description TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_config_templates_tenant ON config_templates(tenant_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_config_templates_name_tenant ON config_templates(tenant_id, name);
