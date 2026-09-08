-- The retention run's count of conference recordings whose registry row
-- it removed (docs/COMPLIANCE.md). The node holding the file normally
-- deletes file and row together; this counts what it left behind.

ALTER TABLE retention_runs
    ADD COLUMN IF NOT EXISTS conference_recordings_deleted BIGINT NOT NULL DEFAULT 0;
