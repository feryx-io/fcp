-- Enterprise calling, phase 1 (docs/ENTERPRISE_CALLING.md §3.1): one row
-- of calling settings per user — do not disturb, call waiting, the four
-- forwards and the ring time — read by every path a call takes to a user.
-- The row is the settings object as JSON, keyed by the user, so the
-- phases that follow add fields without a migration.
CREATE TABLE IF NOT EXISTS user_calling_settings (
    user_id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    settings_json JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_user_calling_settings_tenant
    ON user_calling_settings (tenant_id);

-- A site's slice includes a user's settings (docs/HYBRID_CONTROL_PLANE.md
-- §3.2): the journal names the row by the user.
DROP TRIGGER IF EXISTS sync_journal ON user_calling_settings;
CREATE TRIGGER sync_journal AFTER INSERT OR UPDATE OR DELETE ON user_calling_settings
    FOR EACH ROW EXECUTE FUNCTION fcp_sync_journal('user_id', 'tenant_id');

-- The forwards and the ring time a line binding carried and the call path
-- never applied (only the no-answer forward and its timeout, on the
-- number path) move to the owner's settings: the owner's first binding
-- seeds the row, in the typed target form (`voicemail`, `number:+…`,
-- `ext:…`, a SIP URI as it is). The binding's columns stay until a later
-- contraction; nothing reads them from v0.25.276.
CREATE OR REPLACE FUNCTION fcp_calling_target_of(v TEXT) RETURNS TEXT
LANGUAGE sql IMMUTABLE AS $$
    SELECT CASE
        WHEN v IS NULL OR btrim(v) = '' THEN NULL
        WHEN lower(btrim(v)) = 'voicemail' THEN 'voicemail'
        WHEN btrim(v) LIKE 'sip:%' OR btrim(v) LIKE 'sips:%' THEN btrim(v)
        WHEN btrim(v) ~ '^\+[0-9]{1,15}$' THEN 'number:' || btrim(v)
        WHEN btrim(v) ~ '^[0-9*#]{1,32}$' THEN 'ext:' || btrim(v)
        ELSE NULL
    END
$$;

INSERT INTO user_calling_settings (user_id, tenant_id, settings_json, updated_at)
SELECT DISTINCT ON (owner_user_id)
    owner_user_id,
    tenant_id,
    jsonb_strip_nulls(jsonb_build_object(
        'tenant_id', tenant_id,
        'user_id', owner_user_id,
        'dnd', false,
        'call_waiting', true,
        'no_answer_secs', CASE WHEN no_answer_timeout_secs <> 30
                              THEN LEAST(GREATEST(no_answer_timeout_secs, 5), 300) END,
        'forward_all', fcp_calling_target_of(forward_unconditional),
        'forward_busy', fcp_calling_target_of(forward_on_busy),
        'forward_no_answer', fcp_calling_target_of(forward_on_no_answer)
    )),
    NOW()
FROM line_bindings
WHERE owner_user_id IS NOT NULL
  AND (forward_unconditional IS NOT NULL
       OR forward_on_busy IS NOT NULL
       OR forward_on_no_answer IS NOT NULL
       OR no_answer_timeout_secs <> 30)
ORDER BY owner_user_id, created_at
ON CONFLICT (user_id) DO NOTHING;

DROP FUNCTION fcp_calling_target_of(TEXT);

-- A binding written by a node that no longer knows the column.
ALTER TABLE line_bindings ALTER COLUMN no_answer_timeout_secs SET DEFAULT 30;
