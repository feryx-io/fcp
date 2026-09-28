-- Enterprise calling, phase 9 (docs/ENTERPRISE_CALLING.md §3.9): a shared
-- line. Every device bound to a shared line registers its number as an
-- account of its own and the devices share its appearances, at most
-- `max_appearances` calls at once. The appearances themselves are live
-- state (Redis, or the node's memory), not rows.
ALTER TABLE line_bindings ADD COLUMN IF NOT EXISTS shared BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE line_bindings ADD COLUMN IF NOT EXISTS max_appearances SMALLINT NOT NULL DEFAULT 4
    CHECK (max_appearances BETWEEN 1 AND 24);
