-- Webhook endpoints table
CREATE TABLE IF NOT EXISTS webhook_endpoints (
    id TEXT PRIMARY KEY NOT NULL,
    tenant_id TEXT NOT NULL,
    url TEXT NOT NULL,
    secret TEXT NOT NULL,
    events TEXT NOT NULL DEFAULT '[]',  -- JSON array of event type filters
    enabled INTEGER NOT NULL DEFAULT 1,
    description TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_webhook_endpoints_tenant_id ON webhook_endpoints(tenant_id);
CREATE INDEX IF NOT EXISTS idx_webhook_endpoints_enabled ON webhook_endpoints(tenant_id, enabled);

-- Webhook delivery log table
CREATE TABLE IF NOT EXISTS webhook_delivery_log (
    id TEXT PRIMARY KEY NOT NULL,
    endpoint_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    payload TEXT NOT NULL,
    status_code INTEGER,
    attempts INTEGER NOT NULL DEFAULT 0,
    last_attempt_at INTEGER,
    next_retry_at INTEGER,
    delivered INTEGER NOT NULL DEFAULT 0,
    error TEXT,
    created_at INTEGER NOT NULL,
    FOREIGN KEY (endpoint_id) REFERENCES webhook_endpoints(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_webhook_delivery_log_endpoint ON webhook_delivery_log(endpoint_id);
CREATE INDEX IF NOT EXISTS idx_webhook_delivery_log_pending ON webhook_delivery_log(delivered, next_retry_at)
    WHERE delivered = 0 AND next_retry_at IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_webhook_delivery_log_created ON webhook_delivery_log(created_at);
