-- Migration 011: Queue agent persistence for ACD/Queue system
-- Stores agent login state so agents survive service restarts.

CREATE TABLE IF NOT EXISTS queue_agents (
    tenant_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    extension TEXT NOT NULL,
    queues TEXT NOT NULL,       -- JSON array of queue names
    skills TEXT NOT NULL,       -- JSON array of skill strings
    priority INTEGER NOT NULL DEFAULT 0,
    state TEXT NOT NULL DEFAULT 'available',
    PRIMARY KEY (tenant_id, agent_id)
);
