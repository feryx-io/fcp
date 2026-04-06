-- Queue configuration table for persistent queue CRUD management.
-- Stores queue definitions that can be managed via the REST API.

CREATE TABLE IF NOT EXISTS queue_configs (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    name TEXT NOT NULL,
    extension TEXT NOT NULL,
    strategy TEXT NOT NULL DEFAULT 'round_robin',
    max_wait_time_secs INTEGER DEFAULT 300,
    max_queue_depth INTEGER DEFAULT 50,
    overflow_destination TEXT,
    moh_class TEXT,
    announce_position BOOLEAN DEFAULT true,
    announce_interval_secs INTEGER DEFAULT 30,
    retry_on_no_answer BOOLEAN DEFAULT true,
    agents TEXT[] DEFAULT '{}',
    priority INTEGER DEFAULT 0,
    required_skills TEXT[] DEFAULT '{}',
    record_calls BOOLEAN DEFAULT false,
    callback_enabled BOOLEAN DEFAULT false,
    timezone TEXT DEFAULT 'UTC',
    enabled BOOLEAN DEFAULT true,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW(),
    UNIQUE(tenant_id, name)
);

CREATE INDEX IF NOT EXISTS idx_queue_configs_tenant_id ON queue_configs(tenant_id);
