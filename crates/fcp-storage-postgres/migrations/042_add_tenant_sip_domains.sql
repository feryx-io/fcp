-- Per-tenant SIP domain allowlist used by the SIP tenant resolver.
--
-- Each tenant may claim one or more SIP domains (e.g. "acme.fcp.net",
-- "sip.acme.com"). Incoming REGISTER / INVITE requests whose
-- From/Request-URI host matches one of these entries are routed to
-- that tenant. This replaces the previous tenant-name-as-domain hack
-- and is how the SIP resolver scales to hundreds of tenants.
--
-- Entries are stored lowercase (enforced by the application layer).
-- The GIN index supports fast `WHERE $1 = ANY(sip_domains)` lookups.

ALTER TABLE tenants
    ADD COLUMN IF NOT EXISTS sip_domains TEXT[] NOT NULL DEFAULT ARRAY[]::TEXT[];

CREATE INDEX IF NOT EXISTS idx_tenants_sip_domains ON tenants USING GIN (sip_domains);
