-- System prompts are the platform's (docs/TENANT_SCOPE.md §3.2): a prompt
-- without a tenant, heard by every tenant, overridden by a tenant's prompt of
-- the same name and language.
--
-- Until now the binaries seeded their prompts, marked `is_system`, into their
-- configured default tenant. Those rows become the system prompts. Where more
-- than one tenant holds a system row of one name and language (two binaries
-- configured with different default tenants), the most recently updated one
-- becomes the system prompt and the others stay their tenant's, as overrides.

ALTER TABLE prompts ALTER COLUMN tenant_id DROP NOT NULL;

WITH ranked AS (
    SELECT id,
           row_number() OVER (PARTITION BY name, language ORDER BY updated_at DESC, id) AS rank
    FROM prompts
    WHERE is_system
)
UPDATE prompts p
SET tenant_id = CASE WHEN r.rank = 1 THEN NULL ELSE p.tenant_id END,
    is_system = (r.rank = 1)
FROM ranked r
WHERE p.id = r.id;

-- A tenant's prompts stay unique per tenant by the existing
-- UNIQUE (tenant_id, name, language); NULLs are distinct there, so the system
-- prompts get their own.
CREATE UNIQUE INDEX IF NOT EXISTS prompts_system_name_language
    ON prompts (name, language) WHERE tenant_id IS NULL;

ALTER TABLE prompts ADD CONSTRAINT prompts_system_has_no_tenant
    CHECK (is_system = (tenant_id IS NULL));
