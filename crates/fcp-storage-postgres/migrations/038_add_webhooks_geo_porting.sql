CREATE TABLE IF NOT EXISTS webhook_endpoints (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    endpoint_json JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_webhook_endpoints_tenant
    ON webhook_endpoints (tenant_id, updated_at DESC);

CREATE TABLE IF NOT EXISTS webhook_delivery_logs (
    id TEXT PRIMARY KEY,
    endpoint_id TEXT NOT NULL REFERENCES webhook_endpoints(id) ON DELETE CASCADE,
    delivery_json JSONB NOT NULL,
    delivered BOOLEAN NOT NULL DEFAULT FALSE,
    next_retry_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_webhook_delivery_logs_endpoint
    ON webhook_delivery_logs (endpoint_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_webhook_delivery_logs_pending
    ON webhook_delivery_logs (next_retry_at, created_at)
    WHERE delivered = FALSE AND next_retry_at IS NOT NULL;

CREATE TABLE IF NOT EXISTS geo_routing_rules (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    priority INTEGER NOT NULL DEFAULT 0,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    rule_json JSONB NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_geo_routing_rules_tenant_priority
    ON geo_routing_rules (tenant_id, priority, updated_at DESC);

CREATE TABLE IF NOT EXISTS phone_number_locations (
    phone_number_id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    verified_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    location_json JSONB NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_phone_number_locations_tenant
    ON phone_number_locations (tenant_id, verified_at DESC);

CREATE TABLE IF NOT EXISTS port_requests (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    request_json JSONB NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_port_requests_tenant
    ON port_requests (tenant_id, updated_at DESC);

CREATE INDEX IF NOT EXISTS idx_port_requests_tenant_status
    ON port_requests (tenant_id, status, updated_at DESC);
