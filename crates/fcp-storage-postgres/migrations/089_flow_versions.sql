-- Every version of a flow, kept (docs/DEVELOPER_SURFACE.md phase 7b).
--
-- `flows` holds one row per flow, its latest version: an edit bumps the
-- version in place, so `/flows/:id/versions` returned one row and a flow
-- trace naming version 3 could not be read back. The trigger below
-- records each version as `flows` is written, by whatever path writes it
-- (the admin API, a site's sync, a seed); enabling or disabling a flow
-- does not make a version and only updates the current one's row.

CREATE TABLE IF NOT EXISTS flow_versions (
    flow_id VARCHAR(255) NOT NULL REFERENCES flows(id) ON DELETE CASCADE,
    version INTEGER NOT NULL,
    name VARCHAR(255) NOT NULL,
    description TEXT,
    initial_node VARCHAR(255) NOT NULL,
    nodes JSONB NOT NULL,
    variables JSONB NOT NULL,
    tenant_id VARCHAR(255),
    enabled BOOLEAN NOT NULL,
    -- When this version was written.
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (flow_id, version)
);

CREATE OR REPLACE FUNCTION record_flow_version()
RETURNS TRIGGER AS $$
BEGIN
    INSERT INTO flow_versions (
        flow_id, version, name, description, initial_node, nodes, variables,
        tenant_id, enabled, created_at
    ) VALUES (
        NEW.id, NEW.version, NEW.name, NEW.description, NEW.initial_node,
        NEW.nodes, NEW.variables, NEW.tenant_id, NEW.enabled, NEW.updated_at
    )
    ON CONFLICT (flow_id, version) DO UPDATE SET
        name = EXCLUDED.name,
        description = EXCLUDED.description,
        initial_node = EXCLUDED.initial_node,
        nodes = EXCLUDED.nodes,
        variables = EXCLUDED.variables,
        tenant_id = EXCLUDED.tenant_id,
        enabled = EXCLUDED.enabled;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS flows_record_version ON flows;
CREATE TRIGGER flows_record_version
    AFTER INSERT OR UPDATE ON flows
    FOR EACH ROW
    EXECUTE FUNCTION record_flow_version();

-- The versions there are now: the latest of each flow.
INSERT INTO flow_versions (
    flow_id, version, name, description, initial_node, nodes, variables,
    tenant_id, enabled, created_at
)
SELECT id, version, name, description, initial_node, nodes, variables,
       tenant_id, enabled, updated_at
FROM flows
ON CONFLICT (flow_id, version) DO NOTHING;
