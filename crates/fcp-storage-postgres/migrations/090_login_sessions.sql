-- Sign-in sessions as rows, and users addressed by id
-- (docs/IDENTITY_AND_DEVICES.md phase 1).
--
-- A user's id was indexed but not unique, so nothing could reference it;
-- sessions and API tokens named a user by username, which a deleted and
-- re-created user shares. The id becomes unique (a duplicate, which a v4
-- UUID makes vanishingly unlikely, is given a new one first so the index
-- can be built), and sessions reference it.

UPDATE users SET id = 'user-' || gen_random_uuid()
WHERE ctid IN (
    SELECT ctid FROM (
        SELECT ctid, ROW_NUMBER() OVER (PARTITION BY id ORDER BY created_at) AS n
        FROM users
    ) ranked
    WHERE n > 1
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_users_id_unique ON users(id);

CREATE TABLE IF NOT EXISTS login_sessions (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    -- A deleted user's sessions go with them.
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    -- `password`, later `totp`, `oidc`, `saml`.
    method TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    refreshed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ,
    revoked_reason TEXT,
    ip TEXT,
    user_agent TEXT,
    -- SHA-256 of the current refresh secret, and of the one before it.
    refresh_hash TEXT NOT NULL,
    previous_refresh_hash TEXT
);

CREATE INDEX IF NOT EXISTS idx_login_sessions_user
    ON login_sessions(user_id) WHERE revoked_at IS NULL;
CREATE INDEX IF NOT EXISTS idx_login_sessions_expires ON login_sessions(expires_at);
