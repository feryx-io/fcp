-- Phone number support schema
-- Version: 0.2.0
-- This migration adds tables for phone numbers, line bindings, extensions, and devices

-- Phone numbers table
CREATE TABLE IF NOT EXISTS phone_numbers (
    id TEXT PRIMARY KEY,
    e164 TEXT NOT NULL UNIQUE,
    country_code TEXT NOT NULL,
    npa TEXT NOT NULL,
    nxx TEXT NOT NULL,
    line TEXT NOT NULL,
    state TEXT NOT NULL,
    tenant_id TEXT NOT NULL,

    -- E911/Regulatory
    e911_address_json TEXT, -- JSON serialized E911Address
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
    acquired_at INTEGER NOT NULL,
    activated_at INTEGER,
    ported_at INTEGER,
    loa_date INTEGER,
    foc_date INTEGER,

    -- Features
    recording_enabled INTEGER NOT NULL DEFAULT 0,
    block_anonymous INTEGER NOT NULL DEFAULT 0,
    allow_list TEXT, -- JSON array
    block_list TEXT, -- JSON array
    tags TEXT, -- JSON array

    -- Audit
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_phone_numbers_tenant ON phone_numbers(tenant_id);
CREATE INDEX IF NOT EXISTS idx_phone_numbers_e164 ON phone_numbers(e164);
CREATE INDEX IF NOT EXISTS idx_phone_numbers_state ON phone_numbers(state);

-- Line bindings table (connects phone numbers to users/devices)
CREATE TABLE IF NOT EXISTS line_bindings (
    id TEXT PRIMARY KEY,
    phone_number_id TEXT NOT NULL,
    owner_user_id TEXT, -- NULL for non-user bindings (IVR, queue, etc.)
    binding_type TEXT NOT NULL,
    ring_strategy TEXT NOT NULL,
    tenant_id TEXT NOT NULL,
    no_answer_timeout_secs INTEGER NOT NULL,

    -- Call forwarding
    forward_on_busy TEXT,
    forward_on_no_answer TEXT,
    forward_unconditional TEXT,

    -- Outbound caller ID
    outbound_caller_id TEXT,
    outbound_cnam TEXT,

    -- Audit
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,

    FOREIGN KEY (phone_number_id) REFERENCES phone_numbers(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_line_bindings_phone_number ON line_bindings(phone_number_id);
CREATE INDEX IF NOT EXISTS idx_line_bindings_owner_user ON line_bindings(owner_user_id);
CREATE INDEX IF NOT EXISTS idx_line_bindings_tenant ON line_bindings(tenant_id);

-- Device bindings table (many-to-many between line_bindings and devices)
CREATE TABLE IF NOT EXISTS device_bindings (
    line_binding_id TEXT NOT NULL,
    device_id TEXT NOT NULL,
    priority INTEGER NOT NULL,
    line_appearance INTEGER NOT NULL,
    shared_line INTEGER NOT NULL DEFAULT 0,
    enabled INTEGER NOT NULL DEFAULT 1,

    PRIMARY KEY (line_binding_id, device_id),
    FOREIGN KEY (line_binding_id) REFERENCES line_bindings(id) ON DELETE CASCADE,
    FOREIGN KEY (device_id) REFERENCES devices(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_device_bindings_line ON device_bindings(line_binding_id);
CREATE INDEX IF NOT EXISTS idx_device_bindings_device ON device_bindings(device_id);

-- Extensions table (short codes)
CREATE TABLE IF NOT EXISTS extensions (
    id TEXT PRIMARY KEY,
    number TEXT NOT NULL,
    target_type TEXT NOT NULL, -- "user", "phone_number", "device", "service"
    target_id TEXT NOT NULL,
    tenant_id TEXT NOT NULL,
    description TEXT,
    enabled INTEGER NOT NULL DEFAULT 1,

    -- Audit
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,

    UNIQUE(tenant_id, number)
);

CREATE INDEX IF NOT EXISTS idx_extensions_tenant ON extensions(tenant_id);
CREATE INDEX IF NOT EXISTS idx_extensions_number ON extensions(number);
CREATE INDEX IF NOT EXISTS idx_extensions_target ON extensions(target_type, target_id);

-- Devices table
CREATE TABLE IF NOT EXISTS devices (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    device_type TEXT NOT NULL,
    mac_address TEXT,
    model TEXT,
    firmware_version TEXT,
    tenant_id TEXT NOT NULL,
    owner_user_id TEXT,

    -- SIP configuration
    sip_uri TEXT,
    registration_state TEXT NOT NULL,
    last_registration INTEGER,
    last_ip_address TEXT,
    user_agent TEXT,

    -- Capabilities
    max_lines INTEGER NOT NULL DEFAULT 1,
    supports_video INTEGER NOT NULL DEFAULT 0,
    supports_blf INTEGER NOT NULL DEFAULT 0,
    codec_support TEXT, -- JSON array of codec names

    -- Location/E911
    location TEXT,
    e911_override_json TEXT, -- JSON serialized E911Address

    -- Provisioning
    provisioning_url TEXT,
    config_template_id TEXT,

    -- Status
    enabled INTEGER NOT NULL DEFAULT 1,

    -- Audit
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_devices_tenant ON devices(tenant_id);
CREATE INDEX IF NOT EXISTS idx_devices_owner_user ON devices(owner_user_id);
CREATE INDEX IF NOT EXISTS idx_devices_mac_address ON devices(mac_address);
CREATE INDEX IF NOT EXISTS idx_devices_registration_state ON devices(registration_state);

-- Update users table to add phone number support fields
ALTER TABLE users ADD COLUMN primary_phone_number_id TEXT;
ALTER TABLE users ADD COLUMN voicemail_box_id TEXT;
