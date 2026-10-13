-- Signing out at the identity provider (docs/SSO.md §5): what the
-- provider's logout needs — the ID token for OIDC's id_token_hint, or the
-- SAML NameID — kept with the sign-in until it becomes a session, and
-- with the session.
ALTER TABLE login_sessions ADD COLUMN IF NOT EXISTS idp_logout_hint TEXT;
ALTER TABLE sso_handoffs ADD COLUMN IF NOT EXISTS idp_logout_hint TEXT;
