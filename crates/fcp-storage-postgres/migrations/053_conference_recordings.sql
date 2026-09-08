-- Conference recordings: the registry that makes a recording addressable
-- by tenant, meeting and time rather than by a filename on one node's
-- disk. See docs/VIDEO_CONFERENCING.md §11.

CREATE TABLE IF NOT EXISTS conference_recordings (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    meeting_id TEXT,
    room_number TEXT NOT NULL,
    -- The node holding the file; a download goes through it.
    node_id TEXT NOT NULL,
    path TEXT NOT NULL,
    format TEXT NOT NULL,
    trigger TEXT NOT NULL,
    consent TEXT NOT NULL DEFAULT 'not_required',
    started_at TIMESTAMPTZ NOT NULL,
    -- NULL while the recording is still being written.
    ended_at TIMESTAMPTZ,
    duration_secs BIGINT,
    size_bytes BIGINT,
    participants INTEGER NOT NULL DEFAULT 0
);

-- Listing a tenant's recordings, newest first.
CREATE INDEX IF NOT EXISTS idx_conference_recordings_tenant_started
    ON conference_recordings (tenant_id, started_at DESC);

-- A room's recordings, and the retention sweep's scan.
CREATE INDEX IF NOT EXISTS idx_conference_recordings_room
    ON conference_recordings (tenant_id, room_number, started_at DESC);

-- What a node calls running, for its own restart cleanup.
CREATE INDEX IF NOT EXISTS idx_conference_recordings_running
    ON conference_recordings (node_id)
    WHERE ended_at IS NULL;

-- The node's own sweep: which tenants it holds recordings for, and
-- whether a file on its disk is one the registry knows.
CREATE INDEX IF NOT EXISTS idx_conference_recordings_node_path
    ON conference_recordings (node_id, path);
