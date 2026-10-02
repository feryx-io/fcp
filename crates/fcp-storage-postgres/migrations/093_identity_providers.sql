-- A tenant's identity providers, its users' accounts at them, and sign-ins
-- in flight (docs/IDENTITY_AND_DEVICES.md phase 3). The client secret is
-- sealed at rest like the tenant's other secrets; the encryption job's
-- re-sealing walks `id`.

CREATE TABLE IF NOT EXISTS identity_providers (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    name TEXT NOT NULL,
    kind TEXT NOT NULL DEFAULT 'oidc',
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    settings_json JSONB NOT NULL,
    client_secret TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS identity_providers_tenant ON identity_providers (tenant_id);

-- The provider's subject is the account: a renamed or re-mailed person
-- stays the same user.
CREATE TABLE IF NOT EXISTS user_identities (
    provider_id TEXT NOT NULL REFERENCES identity_providers(id) ON DELETE CASCADE,
    subject TEXT NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    tenant_id TEXT NOT NULL,
    email TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_sign_in_at TIMESTAMPTZ,
    PRIMARY KEY (provider_id, subject)
);
CREATE INDEX IF NOT EXISTS user_identities_user ON user_identities (user_id);

-- A browser's round trip through a provider: the state it carries, the
-- nonce the ID token must repeat and the PKCE verifier, taken once at the
-- callback; then the hash of the one-time code the UI exchanges for a
-- session, taken once.
CREATE TABLE IF NOT EXISTS sso_logins (
    state TEXT PRIMARY KEY,
    provider_id TEXT NOT NULL REFERENCES identity_providers(id) ON DELETE CASCADE,
    tenant_id TEXT NOT NULL,
    nonce TEXT NOT NULL,
    code_verifier TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS sso_handoffs (
    code_hash TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider_id TEXT NOT NULL REFERENCES identity_providers(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
