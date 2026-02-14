-- Conference meetings table
CREATE TABLE IF NOT EXISTS conference_meetings (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    room_number TEXT NOT NULL,
    name TEXT NOT NULL,
    guest_pin TEXT,
    host_pin TEXT,
    require_guest_pin BOOLEAN NOT NULL DEFAULT FALSE,
    max_participants INTEGER NOT NULL DEFAULT 0,
    wait_for_moderator BOOLEAN NOT NULL DEFAULT FALSE,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    auto_record BOOLEAN NOT NULL DEFAULT FALSE,
    default_locked BOOLEAN NOT NULL DEFAULT FALSE,
    permanent BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(tenant_id, room_number)
);

CREATE INDEX IF NOT EXISTS idx_conference_meetings_tenant ON conference_meetings(tenant_id);
CREATE INDEX IF NOT EXISTS idx_conference_meetings_room ON conference_meetings(tenant_id, room_number);
