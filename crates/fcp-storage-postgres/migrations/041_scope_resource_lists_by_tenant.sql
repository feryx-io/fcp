ALTER TABLE resource_lists
    DROP CONSTRAINT IF EXISTS resource_lists_pkey;

ALTER TABLE resource_lists
    ADD CONSTRAINT resource_lists_pkey PRIMARY KEY (tenant_id, uri);
