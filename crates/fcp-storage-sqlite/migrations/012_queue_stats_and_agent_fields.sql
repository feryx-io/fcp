-- Queue stats snapshots for historical analysis
CREATE TABLE IF NOT EXISTS queue_stats_snapshots (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    tenant_id TEXT NOT NULL,
    queue_name TEXT NOT NULL,
    timestamp_epoch INTEGER NOT NULL,
    calls_waiting INTEGER NOT NULL DEFAULT 0,
    agents_available INTEGER NOT NULL DEFAULT 0,
    agents_on_call INTEGER NOT NULL DEFAULT 0,
    agents_total INTEGER NOT NULL DEFAULT 0,
    avg_wait_time_secs REAL NOT NULL DEFAULT 0.0,
    longest_wait_secs INTEGER NOT NULL DEFAULT 0,
    calls_handled_today INTEGER NOT NULL DEFAULT 0,
    calls_abandoned_today INTEGER NOT NULL DEFAULT 0,
    service_level_pct REAL NOT NULL DEFAULT 0.0
);

CREATE INDEX IF NOT EXISTS idx_queue_stats_tenant_queue_ts
    ON queue_stats_snapshots(tenant_id, queue_name, timestamp_epoch);

-- Add missing columns to queue_agents
ALTER TABLE queue_agents ADD COLUMN queue_priorities TEXT NOT NULL DEFAULT '{}';
ALTER TABLE queue_agents ADD COLUMN max_concurrent_calls INTEGER NOT NULL DEFAULT 1;
