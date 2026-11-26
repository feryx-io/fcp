-- A tenant's speech settings (docs/TENANT_SPEECH.md S2): the provider,
-- voice and language the TTS server speaks the tenant's words with when a
-- request names none. One row per tenant; none is the platform's defaults.
CREATE TABLE IF NOT EXISTS tenant_speech (
    tenant_id TEXT PRIMARY KEY REFERENCES tenants(id) ON DELETE CASCADE,
    settings JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
