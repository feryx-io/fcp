-- Initial schema for FCP MySQL storage
-- Version: 0.1.0

-- Users table
CREATE TABLE IF NOT EXISTS users (
    username VARCHAR(255) NOT NULL,
    password_hash TEXT NOT NULL,
    display_name VARCHAR(255),
    tenant_id VARCHAR(255) NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    created_at DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6),
    updated_at DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6) ON UPDATE CURRENT_TIMESTAMP(6),
    media_preferences JSON,
    PRIMARY KEY (tenant_id, username),
    INDEX idx_users_tenant (tenant_id),
    INDEX idx_users_username (username)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

-- Registrations table
CREATE TABLE IF NOT EXISTS registrations (
    aor VARCHAR(255) NOT NULL,
    contact VARCHAR(512) NOT NULL,
    expires_at DATETIME(6) NOT NULL,
    call_id VARCHAR(255) NOT NULL,
    cseq INT NOT NULL,
    user_agent VARCHAR(255),
    q DOUBLE,
    registered_at DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6),
    PRIMARY KEY (aor, contact),
    INDEX idx_registrations_aor (aor),
    INDEX idx_registrations_expires (expires_at)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

-- CDRs table (Call Detail Records)
CREATE TABLE IF NOT EXISTS cdrs (
    call_id VARCHAR(255) PRIMARY KEY,
    tenant_id VARCHAR(255) NOT NULL,
    sip_call_id VARCHAR(255) NOT NULL,

    -- Parties
    from_uri VARCHAR(512) NOT NULL,
    from_display_name VARCHAR(255),
    to_uri VARCHAR(512) NOT NULL,
    to_display_name VARCHAR(255),

    -- Timestamps
    start_time DATETIME(6) NOT NULL,
    answer_time DATETIME(6),
    end_time DATETIME(6),

    -- Durations (milliseconds)
    setup_duration_ms BIGINT,
    call_duration_ms BIGINT,
    total_duration_ms BIGINT,

    -- Call outcome
    disposition VARCHAR(50) NOT NULL,
    hangup_cause VARCHAR(100) NOT NULL,
    hangup_party VARCHAR(50),

    -- SIP details
    sip_status_code INT,
    sip_reason VARCHAR(255),

    -- Routing (future use)
    route_id VARCHAR(255),
    gateway_id VARCHAR(255),

    -- Media information
    media_anchored BOOLEAN NOT NULL DEFAULT FALSE,
    codec_a_leg VARCHAR(50),
    codec_b_leg VARCHAR(50),
    rtp_packets_sent BIGINT,
    rtp_packets_received BIGINT,
    rtp_bytes_sent BIGINT,
    rtp_bytes_received BIGINT,

    -- Recording
    recording_url VARCHAR(1024),

    -- Billing
    cost DECIMAL(10, 4),
    currency VARCHAR(3),

    -- Extensibility
    custom_data JSON,

    INDEX idx_cdrs_tenant (tenant_id),
    INDEX idx_cdrs_start_time (start_time),
    INDEX idx_cdrs_from_uri (from_uri),
    INDEX idx_cdrs_to_uri (to_uri),
    INDEX idx_cdrs_disposition (disposition),
    INDEX idx_cdrs_sip_call_id (sip_call_id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
