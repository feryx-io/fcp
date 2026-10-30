-- Parking lots (docs/PARKING.md, D8 finding 35).
--
-- A tenant's lot: a range of slots numbered by the tenant, as digit
-- strings of one width (`701`–`720`, `050`–`059`), an optional number that
-- parks in the next free slot, how long a call waits and where it goes
-- then. A tenant with no lot parks in the node's `[pbx.parking]` range.
CREATE TABLE IF NOT EXISTS parking_lots (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    name TEXT NOT NULL,
    first_slot TEXT NOT NULL,
    last_slot TEXT NOT NULL,
    park_number TEXT,
    timeout_secs INTEGER NOT NULL DEFAULT 60,
    fallback TEXT,
    music TEXT,
    location_id TEXT,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (char_length(first_slot) = char_length(last_slot)),
    CHECK (first_slot <= last_slot)
);
CREATE INDEX IF NOT EXISTS idx_parking_lots_tenant ON parking_lots (tenant_id, first_slot);
