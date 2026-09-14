-- The CDR archive's bookkeeping (docs/CDR_PIPELINE.md §13, phase 5): each
-- tenant's records for a UTC day are written to one gzipped JSON Lines
-- object; these tables remember which days are written and confirmed, how
-- far the archiver has walked, and which archived days have since been
-- given records (a node's spool draining after an outage).

-- One row per tenant-day the archiver has begun. `archived_at` is set once
-- the target confirms the object and cleared when the day is begun again.
CREATE TABLE IF NOT EXISTS cdr_archives (
    tenant_id TEXT NOT NULL,
    day DATE NOT NULL,
    generation BIGINT NOT NULL DEFAULT 1,
    started_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    archived_at TIMESTAMPTZ,
    object_key TEXT,
    records BIGINT NOT NULL DEFAULT 0,
    bytes BIGINT NOT NULL DEFAULT 0,
    sha256 TEXT,
    PRIMARY KEY (tenant_id, day)
);
CREATE INDEX IF NOT EXISTS idx_cdr_archives_unconfirmed
    ON cdr_archives (day) WHERE archived_at IS NULL;

-- The archiver's walk: every day before `next_day` is enumerated. One row,
-- NULL until the first run. A record insert holds this row in KEY SHARE
-- mode while it notes late records, and moving `next_day` takes it FOR
-- UPDATE, so the walk never passes a day while a record for it is being
-- committed unnoticed.
CREATE TABLE IF NOT EXISTS cdr_archive_state (
    id BOOLEAN PRIMARY KEY DEFAULT true CHECK (id),
    next_day DATE,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
INSERT INTO cdr_archive_state (id) VALUES (true) ON CONFLICT DO NOTHING;

-- Records stored for a day the walk has reached: the day is written again.
-- Append-only (no key, so an insert never waits on another's note); a
-- tenant-day's notes are removed when its write begins.
CREATE TABLE IF NOT EXISTS cdr_archive_late (
    tenant_id TEXT NOT NULL,
    day DATE NOT NULL,
    noted_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_cdr_archive_late ON cdr_archive_late (tenant_id, day);

CREATE OR REPLACE FUNCTION cdr_archive_note_late() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    walked DATE;
BEGIN
    SELECT next_day INTO walked FROM cdr_archive_state FOR KEY SHARE;
    IF walked IS NULL THEN
        RETURN NULL;
    END IF;
    INSERT INTO cdr_archive_late (tenant_id, day)
    SELECT DISTINCT tenant_id, (start_time AT TIME ZONE 'UTC')::date
    FROM new_records
    WHERE (start_time AT TIME ZONE 'UTC')::date <= walked;
    RETURN NULL;
END
$$;

DROP TRIGGER IF EXISTS cdrs_archive_note_late ON cdrs;
CREATE TRIGGER cdrs_archive_note_late
    AFTER INSERT ON cdrs
    REFERENCING NEW TABLE AS new_records
    FOR EACH STATEMENT EXECUTE FUNCTION cdr_archive_note_late();

-- Retention waits for the archive when a tenant's policy says so.
ALTER TABLE compliance_policies
    ADD COLUMN IF NOT EXISTS cdr_archive_required BOOLEAN NOT NULL DEFAULT false;
