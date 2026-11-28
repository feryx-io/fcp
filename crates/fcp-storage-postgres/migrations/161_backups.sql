-- Backups from the admin (docs/SCHEDULED_BACKUPS.md B1): where snapshots go
-- and each run. The repository password, an SFTP private key and an S3
-- secret are sealed at rest; the rest of the target is plain JSON.
CREATE TABLE IF NOT EXISTS backup_destinations (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    target JSONB NOT NULL,
    password TEXT,
    private_key TEXT,
    secret_access_key TEXT,
    schedule JSONB NOT NULL DEFAULT '{}'::jsonb,
    keep JSONB NOT NULL DEFAULT '{}'::jsonb,
    include_key BOOLEAN NOT NULL DEFAULT FALSE,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS backup_runs (
    id TEXT PRIMARY KEY,
    destination_id TEXT NOT NULL REFERENCES backup_destinations(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('backup', 'test')),
    state TEXT NOT NULL CHECK (state IN ('requested', 'running', 'succeeded', 'failed')),
    trigger TEXT NOT NULL,
    requested_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ,
    snapshots TEXT[] NOT NULL DEFAULT '{}',
    bytes_added BIGINT NOT NULL DEFAULT 0,
    message TEXT,
    node TEXT
);
CREATE INDEX IF NOT EXISTS backup_runs_destination ON backup_runs (destination_id, requested_at DESC);
CREATE INDEX IF NOT EXISTS backup_runs_requested ON backup_runs (requested_at) WHERE state = 'requested';
