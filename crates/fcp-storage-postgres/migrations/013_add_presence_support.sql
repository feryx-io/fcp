-- Presence subscriptions
CREATE TABLE IF NOT EXISTS presence_subscriptions (
    subscription_id TEXT PRIMARY KEY,
    watcher_aor TEXT NOT NULL,
    presentity_aor TEXT NOT NULL,
    tenant_id TEXT NOT NULL,
    contact_uri TEXT NOT NULL,
    event_package TEXT NOT NULL DEFAULT 'presence',
    expires_at_epoch BIGINT NOT NULL,
    sip_call_id TEXT,
    sip_from_tag TEXT,
    sip_to_tag TEXT
);
CREATE INDEX IF NOT EXISTS idx_pres_sub_presentity ON presence_subscriptions(presentity_aor);
CREATE INDEX IF NOT EXISTS idx_pres_sub_tenant ON presence_subscriptions(tenant_id);
CREATE INDEX IF NOT EXISTS idx_pres_sub_expires ON presence_subscriptions(expires_at_epoch);

-- Presence presentities
CREATE TABLE IF NOT EXISTS presence_presentities (
    aor TEXT PRIMARY KEY,
    state TEXT NOT NULL,
    note TEXT,
    tenant_id TEXT NOT NULL,
    updated_at_epoch BIGINT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_pres_presentity_tenant ON presence_presentities(tenant_id);

-- Presence auth policies
CREATE TABLE IF NOT EXISTS presence_auth_policies (
    aor TEXT PRIMARY KEY,
    default_allow BOOLEAN NOT NULL DEFAULT TRUE,
    allow_list_json TEXT NOT NULL DEFAULT '[]',
    block_list_json TEXT NOT NULL DEFAULT '[]',
    event_allow_json TEXT NOT NULL DEFAULT '{}',
    event_block_json TEXT NOT NULL DEFAULT '{}',
    time_windows_json TEXT NOT NULL DEFAULT '[]',
    allow_patterns_json TEXT NOT NULL DEFAULT '[]',
    block_patterns_json TEXT NOT NULL DEFAULT '[]',
    tenant_id TEXT NOT NULL
);
