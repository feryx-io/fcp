-- Conference meeting support
-- Version: 010

CREATE TABLE IF NOT EXISTS conference_meetings (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    room_number TEXT NOT NULL,
    name TEXT NOT NULL,
    guest_pin TEXT,
    host_pin TEXT,
    require_guest_pin INTEGER NOT NULL DEFAULT 0,
    max_participants INTEGER NOT NULL DEFAULT 0,
    wait_for_moderator INTEGER NOT NULL DEFAULT 0,
    enabled INTEGER NOT NULL DEFAULT 1,
    auto_record INTEGER NOT NULL DEFAULT 0,
    default_locked INTEGER NOT NULL DEFAULT 0,
    permanent INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE(tenant_id, room_number)
);

CREATE INDEX IF NOT EXISTS idx_conference_meetings_tenant ON conference_meetings(tenant_id);
CREATE INDEX IF NOT EXISTS idx_conference_meetings_room ON conference_meetings(tenant_id, room_number);
