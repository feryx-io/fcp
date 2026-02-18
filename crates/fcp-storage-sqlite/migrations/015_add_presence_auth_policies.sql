-- Migration 015: Presence authorization policies persistence
CREATE TABLE IF NOT EXISTS presence_auth_policies (
    aor TEXT PRIMARY KEY,
    default_allow INTEGER NOT NULL DEFAULT 1,
    allow_list_json TEXT NOT NULL DEFAULT '[]',
    block_list_json TEXT NOT NULL DEFAULT '[]',
    tenant_id TEXT NOT NULL
);
