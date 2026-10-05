-- App sessions and their sign-in in the browser (docs/SOFTPHONE.md §2.2,
-- §2.3): a session belongs to an app install when these are set, and an
-- app's sign-in waits here between the app opening the browser and its
-- redeeming the code.

ALTER TABLE login_sessions
    ADD COLUMN IF NOT EXISTS app_name TEXT,
    ADD COLUMN IF NOT EXISTS app_install_id TEXT,
    ADD COLUMN IF NOT EXISTS app_platform TEXT,
    ADD COLUMN IF NOT EXISTS app_version TEXT;

CREATE INDEX IF NOT EXISTS idx_login_sessions_app_install
    ON login_sessions(user_id, app_install_id)
    WHERE app_install_id IS NOT NULL AND revoked_at IS NULL;

CREATE TABLE IF NOT EXISTS app_authorizations (
    id TEXT PRIMARY KEY,
    client_id TEXT NOT NULL,
    redirect_uri TEXT NOT NULL,
    -- The S256 PKCE challenge the app's verifier must answer.
    code_challenge TEXT NOT NULL,
    state TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL,
    -- Set when the user signs in and grants it.
    user_id TEXT REFERENCES users(id) ON DELETE CASCADE,
    tenant_id TEXT,
    -- SHA-256 of the one-time code, and when it stops being taken.
    code_hash TEXT UNIQUE,
    code_expires_at TIMESTAMPTZ,
    redeemed_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_app_authorizations_expires ON app_authorizations(expires_at);
