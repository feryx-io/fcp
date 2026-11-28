-- The master key downloaded from the backup page (docs/SCHEDULED_BACKUPS.md
-- B2): when, by whom, for which KEK. The page's setup panel stays until the
-- current KEK has been downloaded once; the key itself is never stored here.
CREATE TABLE IF NOT EXISTS backup_key_exports (
    id BIGSERIAL PRIMARY KEY,
    kek_id TEXT NOT NULL,
    actor TEXT NOT NULL,
    exported_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS backup_key_exports_kek ON backup_key_exports (kek_id, exported_at DESC);
