-- SAML sign-in (docs/IDENTITY_AND_DEVICES.md phase 4): every assertion
-- accepted is remembered until it expires, so the same signed assertion
-- posted again — a replay — is refused. Expired ones are cleared as new
-- ones arrive.
CREATE TABLE IF NOT EXISTS saml_assertions (
    provider_id TEXT NOT NULL REFERENCES identity_providers(id) ON DELETE CASCADE,
    assertion_id TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (provider_id, assertion_id)
);
CREATE INDEX IF NOT EXISTS saml_assertions_expiry ON saml_assertions (expires_at);
