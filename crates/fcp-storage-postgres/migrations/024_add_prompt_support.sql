-- Audio prompt management for IVR and system prompts
CREATE TABLE IF NOT EXISTS prompts (
    id              TEXT PRIMARY KEY,
    tenant_id       TEXT NOT NULL,
    name            TEXT NOT NULL,
    description     TEXT,
    category        TEXT NOT NULL CHECK(category IN ('system','ivr','voicemail','queue','greeting','moh','custom')),
    language        TEXT NOT NULL DEFAULT 'en-US',
    source_type     TEXT NOT NULL CHECK(source_type IN ('file', 'tts')),
    tts_text        TEXT,
    tts_voice       TEXT,
    audio_path      TEXT,
    audio_format    TEXT,
    duration_ms     INTEGER,
    file_size       BIGINT,
    is_system       BOOLEAN NOT NULL DEFAULT false,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(tenant_id, name, language)
);

CREATE INDEX IF NOT EXISTS idx_prompts_tenant ON prompts(tenant_id);
CREATE INDEX IF NOT EXISTS idx_prompts_category ON prompts(tenant_id, category);
CREATE INDEX IF NOT EXISTS idx_prompts_name ON prompts(tenant_id, name);
