-- Conference meetings table
CREATE TABLE IF NOT EXISTS conference_meetings (
    id VARCHAR(255) PRIMARY KEY,
    tenant_id VARCHAR(255) NOT NULL,
    room_number VARCHAR(255) NOT NULL,
    name TEXT NOT NULL,
    guest_pin VARCHAR(255),
    host_pin VARCHAR(255),
    require_guest_pin BOOLEAN NOT NULL DEFAULT FALSE,
    max_participants INTEGER NOT NULL DEFAULT 0,
    wait_for_moderator BOOLEAN NOT NULL DEFAULT FALSE,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    auto_record BOOLEAN NOT NULL DEFAULT FALSE,
    default_locked BOOLEAN NOT NULL DEFAULT FALSE,
    permanent BOOLEAN NOT NULL DEFAULT FALSE,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE KEY uq_tenant_room (tenant_id, room_number)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE INDEX idx_conference_meetings_tenant ON conference_meetings(tenant_id);
CREATE INDEX idx_conference_meetings_room ON conference_meetings(tenant_id, room_number);
