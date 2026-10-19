-- The TOML a section was last seeded from (docs/DEEP_TESTING.md D8). A
-- section's row is the source of truth after the first boot, but a key the
-- file changed since must still be read: each boot compares the file with
-- this seed, takes the keys that moved, and keeps the rest of the row, so an
-- edit through the API stands until the file changes that key.
ALTER TABLE config_settings ADD COLUMN IF NOT EXISTS seed JSONB;

-- A row seeded before this column: its last seed is in the audit log.
UPDATE config_settings c
SET seed = (
    SELECT a.new_value
    FROM config_audit_log a
    WHERE a.service = c.service
      AND a.section = c.section
      AND a.tenant_id = c.tenant_id
      AND a.changed_by = 'seed'
    ORDER BY a.changed_at DESC, a.id DESC
    LIMIT 1
)
WHERE c.seed IS NULL;
