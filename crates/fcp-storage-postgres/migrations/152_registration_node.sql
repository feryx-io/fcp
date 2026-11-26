-- The node that took each registration (D8 finding 64): the Registrations
-- page shows it. Unlike flow_node, every binding names one; nothing is
-- routed by it. Rows written before it stay NULL until they re-register.
ALTER TABLE registrations ADD COLUMN IF NOT EXISTS node_id TEXT;
