-- Billing MVP: per-tenant rate plans and longest-prefix rates.
-- A tenant has at most one default (active) plan; calls are rated at CDR
-- creation time against the default plan and the cost/currency stamped onto
-- the existing cdrs.cost / cdrs.currency columns.

-- cdrs.cost was DECIMAL(10,4) but the Rust model reads/writes it as f64
-- (FLOAT8). sqlx cannot decode NUMERIC into f64, which never surfaced only
-- because cost was always NULL until rating landed. Align the column with
-- the code's f64 model before costs start being written.
ALTER TABLE cdrs ALTER COLUMN cost TYPE DOUBLE PRECISION;

CREATE TABLE IF NOT EXISTS rate_plans (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    name TEXT NOT NULL,
    currency TEXT NOT NULL DEFAULT 'USD',
    default_rate_per_minute DOUBLE PRECISION NOT NULL DEFAULT 0,
    connect_fee DOUBLE PRECISION NOT NULL DEFAULT 0,
    billing_increment_secs INTEGER NOT NULL DEFAULT 60
        CHECK (billing_increment_secs > 0),
    is_default BOOLEAN NOT NULL DEFAULT FALSE,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (tenant_id, name)
);

-- At most one default plan per tenant.
CREATE UNIQUE INDEX IF NOT EXISTS idx_rate_plans_tenant_default
    ON rate_plans (tenant_id) WHERE is_default;

CREATE INDEX IF NOT EXISTS idx_rate_plans_tenant ON rate_plans (tenant_id);

CREATE TABLE IF NOT EXISTS rates (
    id TEXT PRIMARY KEY,
    rate_plan_id TEXT NOT NULL REFERENCES rate_plans(id) ON DELETE CASCADE,
    prefix TEXT NOT NULL,
    rate_per_minute DOUBLE PRECISION NOT NULL,
    connect_fee DOUBLE PRECISION,
    description TEXT,
    UNIQUE (rate_plan_id, prefix)
);

CREATE INDEX IF NOT EXISTS idx_rates_plan ON rates (rate_plan_id);
