-- An app install's device (docs/SOFTPHONE.md §2.1): each install that
-- signs in is one of the user's devices, with SIP credentials of its own.
-- The install finds its device here on its next sign-in; the device's
-- removal ends the install's sessions.

CREATE TABLE IF NOT EXISTS app_installs (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    install_id TEXT NOT NULL,
    tenant_id TEXT NOT NULL,
    device_id TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (user_id, install_id)
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_app_installs_device ON app_installs(device_id);

ALTER TABLE login_sessions ADD COLUMN IF NOT EXISTS device_id TEXT;

CREATE INDEX IF NOT EXISTS idx_login_sessions_device
    ON login_sessions(device_id) WHERE device_id IS NOT NULL AND revoked_at IS NULL;
