-- Phone number support schema
-- Version: 0.2.0
-- This migration adds tables for phone numbers, line bindings, extensions, and devices

-- Phone numbers table
CREATE TABLE IF NOT EXISTS phone_numbers (
    id VARCHAR(255) PRIMARY KEY,
    e164 VARCHAR(20) NOT NULL UNIQUE,
    country_code VARCHAR(10) NOT NULL,
    npa VARCHAR(10) NOT NULL,
    nxx VARCHAR(10) NOT NULL,
    line VARCHAR(10) NOT NULL,
    state VARCHAR(50) NOT NULL,
    tenant_id VARCHAR(255) NOT NULL,

    -- E911/Regulatory
    e911_address_json JSON, -- JSON serialized E911Address
    e911_callback TEXT,
    cnam TEXT,
    spam_score REAL,

    -- Carrier metadata
    carrier TEXT,
    lrn TEXT,
    ocn TEXT,
    rate_center TEXT,
    lata TEXT,

    -- Lifecycle
    acquired_at TIMESTAMP NOT NULL,
    activated_at TIMESTAMP NULL,
    ported_at TIMESTAMP NULL,
    loa_date TIMESTAMP NULL,
    foc_date TIMESTAMP NULL,

    -- Features
    recording_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    block_anonymous BOOLEAN NOT NULL DEFAULT FALSE,
    allow_list JSON, -- JSON array
    block_list JSON, -- JSON array
    tags JSON, -- JSON array

    -- Audit
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_phone_numbers_tenant ON phone_numbers(tenant_id);
CREATE INDEX IF NOT EXISTS idx_phone_numbers_e164 ON phone_numbers(e164);
CREATE INDEX IF NOT EXISTS idx_phone_numbers_state ON phone_numbers(state);

-- Line bindings table (connects phone numbers to users/devices)
CREATE TABLE IF NOT EXISTS line_bindings (
    id VARCHAR(255) PRIMARY KEY,
    phone_number_id VARCHAR(255) NOT NULL,
    owner_user_id VARCHAR(255), -- NULL for non-user bindings (IVR, queue, etc.)
    binding_type VARCHAR(50) NOT NULL,
    ring_strategy VARCHAR(50) NOT NULL,
    tenant_id VARCHAR(255) NOT NULL,
    no_answer_timeout_secs INT NOT NULL,

    -- Call forwarding
    forward_on_busy TEXT,
    forward_on_no_answer TEXT,
    forward_unconditional TEXT,

    -- Outbound caller ID
    outbound_caller_id TEXT,
    outbound_cnam TEXT,

    -- Audit
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP,

    FOREIGN KEY (phone_number_id) REFERENCES phone_numbers(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_line_bindings_phone_number ON line_bindings(phone_number_id);
CREATE INDEX IF NOT EXISTS idx_line_bindings_owner_user ON line_bindings(owner_user_id);
CREATE INDEX IF NOT EXISTS idx_line_bindings_tenant ON line_bindings(tenant_id);

-- Device bindings table (many-to-many between line_bindings and devices)
CREATE TABLE IF NOT EXISTS device_bindings (
    line_binding_id VARCHAR(255) NOT NULL,
    device_id VARCHAR(255) NOT NULL,
    priority INT NOT NULL,
    line_appearance INT NOT NULL,
    shared_line BOOLEAN NOT NULL DEFAULT FALSE,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,

    PRIMARY KEY (line_binding_id, device_id),
    FOREIGN KEY (line_binding_id) REFERENCES line_bindings(id) ON DELETE CASCADE,
    FOREIGN KEY (device_id) REFERENCES devices(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_device_bindings_line ON device_bindings(line_binding_id);
CREATE INDEX IF NOT EXISTS idx_device_bindings_device ON device_bindings(device_id);

-- Extensions table (short codes)
CREATE TABLE IF NOT EXISTS extensions (
    id VARCHAR(255) PRIMARY KEY,
    number VARCHAR(20) NOT NULL,
    target_type VARCHAR(50) NOT NULL, -- "user", "phone_number", "device", "service"
    target_id VARCHAR(255) NOT NULL,
    tenant_id VARCHAR(255) NOT NULL,
    description TEXT,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,

    -- Audit
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP,

    UNIQUE(tenant_id, number)
);

CREATE INDEX IF NOT EXISTS idx_extensions_tenant ON extensions(tenant_id);
CREATE INDEX IF NOT EXISTS idx_extensions_number ON extensions(number);
CREATE INDEX IF NOT EXISTS idx_extensions_target ON extensions(target_type, target_id);

-- Devices table
CREATE TABLE IF NOT EXISTS devices (
    id VARCHAR(255) PRIMARY KEY,
    name TEXT NOT NULL,
    device_type VARCHAR(50) NOT NULL,
    mac_address VARCHAR(20),
    model VARCHAR(100),
    firmware_version VARCHAR(50),
    tenant_id VARCHAR(255) NOT NULL,
    owner_user_id VARCHAR(255),

    -- SIP configuration
    sip_uri TEXT,
    registration_state VARCHAR(50) NOT NULL,
    last_registration TIMESTAMP NULL,
    last_ip_address TEXT,
    user_agent TEXT,

    -- Capabilities
    max_lines INT NOT NULL DEFAULT 1,
    supports_video BOOLEAN NOT NULL DEFAULT FALSE,
    supports_blf BOOLEAN NOT NULL DEFAULT FALSE,
    codec_support JSON, -- JSON array of codec names

    -- Location/E911
    location TEXT,
    e911_override_json JSON, -- JSON serialized E911Address

    -- Provisioning
    provisioning_url TEXT,
    config_template_id TEXT,

    -- Status
    enabled BOOLEAN NOT NULL DEFAULT TRUE,

    -- Audit
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_devices_tenant ON devices(tenant_id);
CREATE INDEX IF NOT EXISTS idx_devices_owner_user ON devices(owner_user_id);
CREATE INDEX IF NOT EXISTS idx_devices_mac_address ON devices(mac_address);
CREATE INDEX IF NOT EXISTS idx_devices_registration_state ON devices(registration_state);

-- Update users table to add phone number support fields
ALTER TABLE users ADD COLUMN primary_phone_number_id VARCHAR(255);
ALTER TABLE users ADD COLUMN voicemail_box_id TEXT;
