-- Initial schema for FCP PostgreSQL storage
-- Version: 0.1.0

-- Users table
CREATE TABLE IF NOT EXISTS users (
    username TEXT NOT NULL,
    password_hash TEXT NOT NULL,
    display_name TEXT,
    tenant_id TEXT NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    media_preferences JSONB,
    PRIMARY KEY (tenant_id, username)
);

CREATE INDEX IF NOT EXISTS idx_users_tenant ON users(tenant_id);
CREATE INDEX IF NOT EXISTS idx_users_username ON users(username);
CREATE INDEX IF NOT EXISTS idx_users_media_prefs ON users USING GIN (media_preferences);

-- Registrations table
CREATE TABLE IF NOT EXISTS registrations (
    aor TEXT NOT NULL,
    contact TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    call_id TEXT NOT NULL,
    cseq INTEGER NOT NULL,
    user_agent TEXT,
    q REAL,
    registered_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (aor, contact)
);

CREATE INDEX IF NOT EXISTS idx_registrations_aor ON registrations(aor);
CREATE INDEX IF NOT EXISTS idx_registrations_expires ON registrations(expires_at);

-- CDRs table (Call Detail Records)
CREATE TABLE IF NOT EXISTS cdrs (
    call_id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    sip_call_id TEXT NOT NULL,

    -- Parties
    from_uri TEXT NOT NULL,
    from_display_name TEXT,
    to_uri TEXT NOT NULL,
    to_display_name TEXT,

    -- Timestamps
    start_time TIMESTAMPTZ NOT NULL,
    answer_time TIMESTAMPTZ,
    end_time TIMESTAMPTZ,

    -- Durations (milliseconds)
    setup_duration_ms BIGINT,
    call_duration_ms BIGINT,
    total_duration_ms BIGINT,

    -- Call outcome
    disposition TEXT NOT NULL,
    hangup_cause TEXT NOT NULL,
    hangup_party TEXT,

    -- SIP details
    sip_status_code INTEGER,
    sip_reason TEXT,

    -- Routing (future use)
    route_id TEXT,
    gateway_id TEXT,

    -- Media information
    media_anchored BOOLEAN NOT NULL DEFAULT false,
    codec_a_leg TEXT,
    codec_b_leg TEXT,
    rtp_packets_sent BIGINT,
    rtp_packets_received BIGINT,
    rtp_bytes_sent BIGINT,
    rtp_bytes_received BIGINT,

    -- Recording
    recording_url TEXT,

    -- Billing
    cost DECIMAL(10, 4),
    currency TEXT,

    -- Extensibility
    custom_data JSONB
);

CREATE INDEX IF NOT EXISTS idx_cdrs_tenant ON cdrs(tenant_id);
CREATE INDEX IF NOT EXISTS idx_cdrs_start_time ON cdrs(start_time);
CREATE INDEX IF NOT EXISTS idx_cdrs_from_uri ON cdrs(from_uri);
CREATE INDEX IF NOT EXISTS idx_cdrs_to_uri ON cdrs(to_uri);
CREATE INDEX IF NOT EXISTS idx_cdrs_disposition ON cdrs(disposition);
CREATE INDEX IF NOT EXISTS idx_cdrs_sip_call_id ON cdrs(sip_call_id);
CREATE INDEX IF NOT EXISTS idx_cdrs_custom_data ON cdrs USING GIN (custom_data);

-- Optional: Partitioning setup for CDRs (commented out, can be enabled for large deployments)
-- ALTER TABLE cdrs PARTITION BY RANGE (start_time);
-- CREATE TABLE cdrs_2026_01 PARTITION OF cdrs FOR VALUES FROM ('2026-01-01') TO ('2026-02-01');
