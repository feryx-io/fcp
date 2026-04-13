CREATE TABLE IF NOT EXISTS presence_history (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    aor TEXT NOT NULL,
    old_state TEXT NOT NULL,
    new_state TEXT NOT NULL,
    timestamp_epoch BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_presence_history_tenant_aor_ts
    ON presence_history(tenant_id, aor, timestamp_epoch DESC);

CREATE INDEX IF NOT EXISTS idx_presence_history_tenant_ts
    ON presence_history(tenant_id, timestamp_epoch DESC);
