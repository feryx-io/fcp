CREATE TABLE IF NOT EXISTS presence_subscriptions (
    subscription_id TEXT PRIMARY KEY,
    watcher_aor TEXT NOT NULL,
    presentity_aor TEXT NOT NULL,
    tenant_id TEXT NOT NULL,
    contact_uri TEXT NOT NULL,
    event_package TEXT NOT NULL DEFAULT 'presence',
    expires_at_epoch INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_presence_sub_presentity ON presence_subscriptions(presentity_aor);
CREATE INDEX IF NOT EXISTS idx_presence_sub_tenant ON presence_subscriptions(tenant_id);
CREATE INDEX IF NOT EXISTS idx_presence_sub_expires ON presence_subscriptions(expires_at_epoch);
