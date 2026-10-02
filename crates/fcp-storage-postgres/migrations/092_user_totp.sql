-- A user's second factor (docs/IDENTITY_AND_DEVICES.md phase 2b): a TOTP
-- secret, sealed at rest like the user's other secrets, the last step a
-- code was used at (a code is good once), and the hashes of the recovery
-- codes not yet spent. Keyed by the user's id, so the encryption job's
-- re-sealing (which walks `id`) reaches it.

CREATE TABLE IF NOT EXISTS user_totp (
    id TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    tenant_id TEXT NOT NULL,
    secret TEXT NOT NULL,
    confirmed_at TIMESTAMPTZ,
    last_step BIGINT,
    recovery_hashes TEXT[] NOT NULL DEFAULT ARRAY[]::TEXT[],
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
