-- A contact's tenant is its owner's, whether or not the tenant has a row:
-- a single-tenant deployment's default tenant need not have one, and its
-- users' contacts failed to save (D8). The owner's foreign key still
-- removes a user's contacts with the user, as for every other per-user
-- table, none of which ties tenant_id to tenants.
ALTER TABLE personal_contacts DROP CONSTRAINT IF EXISTS personal_contacts_tenant_id_fkey;
