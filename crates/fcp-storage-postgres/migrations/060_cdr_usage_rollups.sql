-- Hourly usage rollups (docs/CDR_PIPELINE.md §9): what the usage and
-- billing summary routes read for closed hours, rebuilt — recomputed, not
-- incremented — by the maintenance job, so a replayed record cannot count
-- twice.

CREATE TABLE IF NOT EXISTS cdr_usage_hourly (
    tenant_id TEXT NOT NULL,
    hour TIMESTAMPTZ NOT NULL,
    kind TEXT NOT NULL,
    direction TEXT NOT NULL,
    calls BIGINT NOT NULL,
    answered BIGINT NOT NULL,
    duration_ms BIGINT NOT NULL,
    cost NUMERIC(18,6) NOT NULL,
    PRIMARY KEY (tenant_id, hour, kind, direction)
);
CREATE INDEX IF NOT EXISTS idx_cdr_usage_hourly_hour ON cdr_usage_hourly (hour);

CREATE TABLE IF NOT EXISTS cdr_number_usage_hourly (
    tenant_id TEXT NOT NULL,
    hour TIMESTAMPTZ NOT NULL,
    number TEXT NOT NULL,
    calls BIGINT NOT NULL,
    answered BIGINT NOT NULL,
    duration_ms BIGINT NOT NULL,
    inbound BIGINT NOT NULL,
    outbound BIGINT NOT NULL,
    PRIMARY KEY (tenant_id, hour, number)
);
CREATE INDEX IF NOT EXISTS idx_cdr_number_usage_hourly_hour ON cdr_number_usage_hourly (hour);

-- Which hours the rollups cover: [covers_from, covers_until). Absent until
-- the first rebuild, when every read goes to the records.
CREATE TABLE IF NOT EXISTS cdr_rollup_state (
    name TEXT PRIMARY KEY,
    covers_from TIMESTAMPTZ NOT NULL,
    covers_until TIMESTAMPTZ NOT NULL,
    rebuilt_at TIMESTAMPTZ NOT NULL
);
