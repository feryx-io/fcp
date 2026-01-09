-- Initial schema for FCP SQLite storage
-- Version: 0.1.0

-- Users table
CREATE TABLE IF NOT EXISTS users (
    username TEXT NOT NULL,
    password_hash TEXT NOT NULL,
    display_name TEXT,
    tenant_id TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    media_preferences TEXT, -- JSON
    PRIMARY KEY (tenant_id, username)
);

CREATE INDEX IF NOT EXISTS idx_users_tenant ON users(tenant_id);
CREATE INDEX IF NOT EXISTS idx_users_username ON users(username);

-- Registrations table
CREATE TABLE IF NOT EXISTS registrations (
    aor TEXT NOT NULL,
    contact TEXT NOT NULL,
    expires_at INTEGER NOT NULL,
    call_id TEXT NOT NULL,
    cseq INTEGER NOT NULL,
    user_agent TEXT,
    q REAL,
    registered_at INTEGER NOT NULL,
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

    -- Timestamps (Unix epoch seconds)
    start_time INTEGER NOT NULL,
    answer_time INTEGER,
    end_time INTEGER,

    -- Durations (milliseconds)
    setup_duration_ms INTEGER,
    call_duration_ms INTEGER,
    total_duration_ms INTEGER,

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
    media_anchored INTEGER NOT NULL DEFAULT 0,
    codec_a_leg TEXT,
    codec_b_leg TEXT,
    rtp_packets_sent INTEGER,
    rtp_packets_received INTEGER,
    rtp_bytes_sent INTEGER,
    rtp_bytes_received INTEGER,

    -- Recording
    recording_url TEXT,

    -- Billing
    cost REAL,
    currency TEXT,

    -- Extensibility
    custom_data TEXT -- JSON
);

CREATE INDEX IF NOT EXISTS idx_cdrs_tenant ON cdrs(tenant_id);
CREATE INDEX IF NOT EXISTS idx_cdrs_start_time ON cdrs(start_time);
CREATE INDEX IF NOT EXISTS idx_cdrs_from_uri ON cdrs(from_uri);
CREATE INDEX IF NOT EXISTS idx_cdrs_to_uri ON cdrs(to_uri);
CREATE INDEX IF NOT EXISTS idx_cdrs_disposition ON cdrs(disposition);
CREATE INDEX IF NOT EXISTS idx_cdrs_sip_call_id ON cdrs(sip_call_id);
