-- More backup destinations and the watch (docs/SCHEDULED_BACKUPS.md B4): a
-- directory, a REST server and rclone; the secret a REST server or an rclone
-- remote needs, sealed; the weekly check as a run; and a destination's
-- overdue noticed once per lapse.
ALTER TABLE backup_destinations ADD COLUMN IF NOT EXISTS target_secret TEXT;

ALTER TABLE backup_runs DROP CONSTRAINT IF EXISTS backup_runs_kind_check;
ALTER TABLE backup_runs ADD CONSTRAINT backup_runs_kind_check
    CHECK (kind IN ('backup', 'test', 'check'));

CREATE TABLE IF NOT EXISTS backup_overdue_notices (
    destination_id TEXT PRIMARY KEY REFERENCES backup_destinations(id) ON DELETE CASCADE,
    -- The last good backup it was overdue from; none if it never had one.
    last_success_at TIMESTAMPTZ,
    noticed_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
