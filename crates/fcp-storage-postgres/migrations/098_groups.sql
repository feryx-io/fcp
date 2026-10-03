-- A tenant's groups of users (docs/USERS.md §5, docs/IDENTITY_AND_DEVICES.md
-- phase 5c): a group may give its members a role, may follow a group at the
-- tenant's identity providers, and may be a queue's agents.

CREATE TABLE IF NOT EXISTS groups (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    name TEXT NOT NULL,
    description TEXT,
    role TEXT,
    idp_group TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_groups_tenant_name ON groups (tenant_id, lower(name));

-- A membership goes with its group or its user. The journal names a row by
-- one column: the group and the user.
CREATE TABLE IF NOT EXISTS group_members (
    group_id TEXT NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    tenant_id TEXT NOT NULL,
    added_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    id TEXT GENERATED ALWAYS AS (group_id || ':' || user_id) STORED,
    PRIMARY KEY (group_id, user_id)
);
CREATE INDEX IF NOT EXISTS idx_group_members_user ON group_members (user_id);
CREATE INDEX IF NOT EXISTS idx_group_members_tenant ON group_members (tenant_id);

-- A site's users hold their groups' roles there too (docs/HYBRID_CONTROL_PLANE.md §3.2).
DROP TRIGGER IF EXISTS sync_journal ON groups;
CREATE TRIGGER sync_journal AFTER INSERT OR UPDATE OR DELETE ON groups
    FOR EACH ROW EXECUTE FUNCTION fcp_sync_journal('id', 'tenant_id');

DROP TRIGGER IF EXISTS sync_journal ON group_members;
CREATE TRIGGER sync_journal AFTER INSERT OR UPDATE OR DELETE ON group_members
    FOR EACH ROW EXECUTE FUNCTION fcp_sync_journal('id', 'tenant_id');

-- A queue whose agents are a group's members.
ALTER TABLE queue_configs ADD COLUMN IF NOT EXISTS agent_group_id TEXT;
