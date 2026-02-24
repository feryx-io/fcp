-- Configuration settings table: stores operational config as JSONB per section.
-- Uses a "DB primary, TOML seed" pattern where TOML values seed on first boot.
CREATE TABLE IF NOT EXISTS config_settings (
    id          BIGSERIAL PRIMARY KEY,
    service     TEXT NOT NULL,                        -- "call-manager", "admin", "_global"
    section     TEXT NOT NULL,                        -- "sip", "registrar", "media"
    tenant_id   TEXT NOT NULL DEFAULT '_system',      -- "_system" for global
    value       JSONB NOT NULL,                       -- section config as JSON
    version     BIGINT NOT NULL DEFAULT 1,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_by  TEXT,
    UNIQUE (service, section, tenant_id)
);

CREATE INDEX IF NOT EXISTS idx_config_settings_service
    ON config_settings (service);
CREATE INDEX IF NOT EXISTS idx_config_settings_tenant
    ON config_settings (tenant_id);
CREATE INDEX IF NOT EXISTS idx_config_settings_service_tenant
    ON config_settings (service, tenant_id);

-- Audit log for configuration changes.
CREATE TABLE IF NOT EXISTS config_audit_log (
    id          BIGSERIAL PRIMARY KEY,
    service     TEXT NOT NULL,
    section     TEXT NOT NULL,
    tenant_id   TEXT NOT NULL,
    old_value   JSONB,
    new_value   JSONB NOT NULL,
    changed_by  TEXT,
    changed_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    version     BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_config_audit_service
    ON config_audit_log (service);
CREATE INDEX IF NOT EXISTS idx_config_audit_service_section
    ON config_audit_log (service, section);
CREATE INDEX IF NOT EXISTS idx_config_audit_changed_at
    ON config_audit_log (changed_at DESC);
