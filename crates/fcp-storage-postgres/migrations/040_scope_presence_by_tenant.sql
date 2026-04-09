ALTER TABLE presence_presentities
    DROP CONSTRAINT IF EXISTS presence_presentities_pkey;

ALTER TABLE presence_presentities
    ADD CONSTRAINT presence_presentities_pkey PRIMARY KEY (tenant_id, aor);

ALTER TABLE presence_auth_policies
    DROP CONSTRAINT IF EXISTS presence_auth_policies_pkey;

ALTER TABLE presence_auth_policies
    ADD CONSTRAINT presence_auth_policies_pkey PRIMARY KEY (tenant_id, aor);
