-- A person's calls (docs/SOFTPHONE.md §2.7, GET /me/calls): the records
-- whose caller or callee has one of their canonical keys, found through
-- this index rather than by reading every record of the tenant. On the
-- partitioned parent, so each monthly partition gets its own.
CREATE INDEX IF NOT EXISTS idx_cdrs_subject_keys ON cdrs
    USING gin ((fcp_subject_keys(from_uri) || fcp_subject_keys(to_uri)));

-- What a tenant's people see of each other (GET /me/directory): on unless
-- the tenant turns it off.
ALTER TABLE tenants
    ADD COLUMN IF NOT EXISTS directory_json JSONB NOT NULL
    DEFAULT '{"visible_to_users": true}'::jsonb;
