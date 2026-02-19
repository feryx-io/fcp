-- Presence subscriptions
CREATE TABLE IF NOT EXISTS presence_subscriptions (
    subscription_id VARCHAR(255) PRIMARY KEY,
    watcher_aor VARCHAR(255) NOT NULL,
    presentity_aor VARCHAR(255) NOT NULL,
    tenant_id VARCHAR(255) NOT NULL,
    contact_uri TEXT NOT NULL,
    event_package VARCHAR(64) NOT NULL DEFAULT 'presence',
    expires_at_epoch BIGINT NOT NULL,
    sip_call_id VARCHAR(255),
    sip_from_tag VARCHAR(255),
    sip_to_tag VARCHAR(255),
    INDEX idx_pres_sub_presentity (presentity_aor),
    INDEX idx_pres_sub_tenant (tenant_id),
    INDEX idx_pres_sub_expires (expires_at_epoch)
);

-- Presence presentities
CREATE TABLE IF NOT EXISTS presence_presentities (
    aor VARCHAR(255) PRIMARY KEY,
    state VARCHAR(64) NOT NULL,
    note TEXT,
    tenant_id VARCHAR(255) NOT NULL,
    updated_at_epoch BIGINT NOT NULL,
    INDEX idx_pres_presentity_tenant (tenant_id)
);

-- Presence auth policies
CREATE TABLE IF NOT EXISTS presence_auth_policies (
    aor VARCHAR(255) PRIMARY KEY,
    default_allow TINYINT(1) NOT NULL DEFAULT 1,
    allow_list_json TEXT NOT NULL,
    block_list_json TEXT NOT NULL,
    event_allow_json TEXT NOT NULL,
    event_block_json TEXT NOT NULL,
    time_windows_json TEXT NOT NULL,
    allow_patterns_json TEXT NOT NULL,
    block_patterns_json TEXT NOT NULL,
    tenant_id VARCHAR(255) NOT NULL
);
