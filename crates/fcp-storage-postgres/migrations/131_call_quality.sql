-- A call's quality as a phone measured it (docs/CALL_DIAGNOSTICS.md §2):
-- the final figures an app sends when a call ends, one report per phone on
-- the call, kept beside the call's record and gone with it.
CREATE TABLE IF NOT EXISTS call_quality (
    tenant_id TEXT NOT NULL,
    call_id TEXT NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    -- The app's device; '' for a report from no device.
    device_id TEXT NOT NULL DEFAULT '',
    direction TEXT NOT NULL,
    codec TEXT NOT NULL,
    srtp BOOLEAN NOT NULL,
    duration_seconds INTEGER,
    packets_sent BIGINT NOT NULL,
    packets_received BIGINT NOT NULL,
    packets_expected BIGINT NOT NULL,
    packets_lost BIGINT NOT NULL,
    loss_pct REAL NOT NULL,
    jitter_ms REAL NOT NULL,
    rtt_ms REAL,
    far_loss_pct REAL,
    far_jitter_ms REAL,
    far_lost BIGINT,
    mos REAL NOT NULL,
    far_mos REAL,
    local_rtp TEXT,
    remote_rtp TEXT,
    reported_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, call_id, user_id, device_id)
);
CREATE INDEX IF NOT EXISTS idx_call_quality_user ON call_quality (user_id, call_id);
CREATE INDEX IF NOT EXISTS idx_call_quality_reported ON call_quality (tenant_id, reported_at);
