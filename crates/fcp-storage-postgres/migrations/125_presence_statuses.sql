-- Presence a person sets (docs/PRESENCE.md, D8 finding 33).
--
-- The status a user chose: a built-in state's name (available, away,
-- busy, dnd) or one of their own statuses' ids, with a note and a time
-- it clears at. No row is Available. Others see it with calls, do not
-- disturb, idleness and being signed out laid over it, worked out by the
-- admin; none of that is stored.
CREATE TABLE IF NOT EXISTS user_presence (
    user_id TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    tenant_id TEXT NOT NULL,
    status TEXT NOT NULL,
    note TEXT,
    until TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_user_presence_tenant ON user_presence (tenant_id);

-- A user's own statuses: a label ("WFH") on one of the four states, in
-- the user's order.
CREATE TABLE IF NOT EXISTS presence_statuses (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    label TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('available', 'away', 'busy', 'dnd')),
    position INTEGER NOT NULL DEFAULT 0,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_presence_statuses_user
    ON presence_statuses (user_id, position, id);
