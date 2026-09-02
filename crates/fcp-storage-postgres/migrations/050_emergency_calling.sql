-- E911 / emergency calling.
--
-- Trunks flagged emergency_capable are tried first for 911 calls; site
-- locations carry the ELIN (callback number) the carrier maps to their
-- address; emergency_configs holds per-tenant numbers and notification
-- contacts; emergency_calls is the audit trail (RAY BAUM's Act record
-- keeping, Kari's Law notification) written when an emergency call is
-- placed.

ALTER TABLE trunks ADD COLUMN IF NOT EXISTS emergency_capable BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE locations ADD COLUMN IF NOT EXISTS e911_callback_number TEXT;

CREATE TABLE IF NOT EXISTS emergency_configs (
    tenant_id TEXT PRIMARY KEY,
    emergency_numbers TEXT[] NOT NULL DEFAULT ARRAY['911'],
    test_numbers TEXT[] NOT NULL DEFAULT ARRAY['933'],
    dial_prefixes TEXT[] NOT NULL DEFAULT ARRAY['9'],
    notify_contacts TEXT[] NOT NULL DEFAULT ARRAY[]::TEXT[],
    require_dispatchable_location BOOLEAN NOT NULL DEFAULT FALSE,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS emergency_calls (
    id BIGSERIAL PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    call_id TEXT NOT NULL,
    sip_call_id TEXT,
    caller_uri TEXT NOT NULL,
    caller_username TEXT,
    dialed_number TEXT NOT NULL,
    kind TEXT NOT NULL,
    callback_number TEXT,
    location JSONB,
    location_source TEXT,
    trunk_name TEXT,
    outcome TEXT NOT NULL,
    placed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    notified BOOLEAN NOT NULL DEFAULT FALSE
);

CREATE INDEX IF NOT EXISTS idx_emergency_calls_tenant_placed
    ON emergency_calls (tenant_id, placed_at DESC);
CREATE INDEX IF NOT EXISTS idx_emergency_calls_call_id
    ON emergency_calls (call_id);
