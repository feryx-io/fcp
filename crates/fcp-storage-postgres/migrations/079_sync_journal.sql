-- The sync journal (docs/HYBRID_CONTROL_PLANE.md §3.2, phase 2): every
-- change to a table a site needs, numbered, per tenant, written by a
-- trigger so that no code path, migration or hand-run SQL can change a
-- synced row without a site's link hearing of it. The row itself is not
-- here: a link reads the current object by (table, id) when it applies
-- the entry, and a row gone by then is a delete.
CREATE TABLE IF NOT EXISTS sync_journal (
    seq        BIGSERIAL PRIMARY KEY,
    tenant_id  TEXT NOT NULL,
    table_name TEXT NOT NULL,
    row_id     TEXT NOT NULL,
    op         TEXT NOT NULL,
    at         TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_sync_journal_tenant_seq ON sync_journal (tenant_id, seq);

-- TG_ARGV[0]: the column that names the row; TG_ARGV[1]: the column that
-- names the tenant ('id' on tenants themselves, 'rate_plan_id' on rates,
-- whose tenant is their plan's). A row that names no tenant is nobody's
-- to sync and is skipped.
CREATE OR REPLACE FUNCTION fcp_sync_journal() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    row_json JSONB;
    tenant   TEXT;
    rid      TEXT;
    op       TEXT;
BEGIN
    IF TG_OP = 'DELETE' THEN
        row_json := to_jsonb(OLD);
        op := 'delete';
    ELSE
        row_json := to_jsonb(NEW);
        op := 'put';
    END IF;
    rid := row_json ->> TG_ARGV[0];
    IF TG_ARGV[1] = 'rate_plan_id' THEN
        SELECT tenant_id INTO tenant FROM rate_plans WHERE id = row_json ->> 'rate_plan_id';
    ELSE
        tenant := row_json ->> TG_ARGV[1];
    END IF;
    IF tenant IS NULL OR rid IS NULL THEN
        RETURN NULL;
    END IF;
    INSERT INTO sync_journal (tenant_id, table_name, row_id, op)
    VALUES (tenant, TG_TABLE_NAME, rid, op);
    RETURN NULL;
END
$$;

DO $$
DECLARE
    t TEXT;
BEGIN
    FOREACH t IN ARRAY ARRAY[
        'users', 'devices', 'extensions', 'line_bindings', 'phone_numbers',
        'routing_rules', 'lcr_routes', 'trunks', 'queue_configs', 'flows',
        'prompts', 'config_templates', 'conference_meetings', 'locations',
        'emergency_networks', 'rate_plans'
    ] LOOP
        EXECUTE format('DROP TRIGGER IF EXISTS sync_journal ON %I', t);
        EXECUTE format(
            'CREATE TRIGGER sync_journal AFTER INSERT OR UPDATE OR DELETE ON %I
             FOR EACH ROW EXECUTE FUNCTION fcp_sync_journal(''id'', ''tenant_id'')', t);
    END LOOP;
END
$$;

DROP TRIGGER IF EXISTS sync_journal ON tenants;
CREATE TRIGGER sync_journal AFTER INSERT OR UPDATE OR DELETE ON tenants
    FOR EACH ROW EXECUTE FUNCTION fcp_sync_journal('id', 'id');

DROP TRIGGER IF EXISTS sync_journal ON emergency_configs;
CREATE TRIGGER sync_journal AFTER INSERT OR UPDATE OR DELETE ON emergency_configs
    FOR EACH ROW EXECUTE FUNCTION fcp_sync_journal('tenant_id', 'tenant_id');

-- A rate is part of its plan: the entry names the plan, and a link
-- re-reads the plan with its rates.
DROP TRIGGER IF EXISTS sync_journal ON rates;
CREATE TRIGGER sync_journal AFTER INSERT OR UPDATE OR DELETE ON rates
    FOR EACH ROW EXECUTE FUNCTION fcp_sync_journal('rate_plan_id', 'rate_plan_id');
