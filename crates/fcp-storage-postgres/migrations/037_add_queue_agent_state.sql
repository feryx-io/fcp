-- Persistent queue agent state for ACD login/pause/priority restoration.

CREATE TABLE IF NOT EXISTS queue_agents (
    tenant_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    extension TEXT NOT NULL,
    queues TEXT[] NOT NULL DEFAULT '{}',
    skills TEXT[] NOT NULL DEFAULT '{}',
    priority INTEGER NOT NULL DEFAULT 0,
    state TEXT NOT NULL DEFAULT 'logged_out',
    queue_priorities JSONB NOT NULL DEFAULT '{}'::jsonb,
    max_concurrent_calls INTEGER NOT NULL DEFAULT 1,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, agent_id)
);

CREATE INDEX IF NOT EXISTS idx_queue_agents_tenant_id ON queue_agents(tenant_id);
