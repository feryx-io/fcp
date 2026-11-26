-- A tenant's own TTS vendor accounts (docs/TENANT_SPEECH.md S3): bring your
-- own key. The secret (API key, subscription key or AWS secret access key)
-- is sealed at rest like an identity provider's client secret.
CREATE TABLE IF NOT EXISTS speech_accounts (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    kind TEXT NOT NULL,
    settings JSONB NOT NULL DEFAULT '{}'::jsonb,
    secret TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, name)
);
CREATE INDEX IF NOT EXISTS speech_accounts_tenant ON speech_accounts (tenant_id);
