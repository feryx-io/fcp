-- The CDR pipeline, phase 4 (docs/CDR_PIPELINE.md §8): `cdrs` becomes a
-- table partitioned by month of start_time.
--
-- Postgres requires the partition key in every unique constraint, so the
-- record's key is (cdr_id, start_time) — a replay has both — and the
-- one-call-record rule is (call_id, start_time) for call records.
--
-- The copy is one statement in this migration's transaction: the table is
-- locked while it runs, and writers wait in their spools (measured in the
-- design document). The key and the indexes are built after the copy,
-- each in one sorted pass, rather than kept up row by row during it, which
-- on a large table is many times slower. A default partition takes any row
-- outside the monthly ones; the storage layer creates the coming months on
-- start and daily, moving rows out of the default partition when it does.

SET LOCAL maintenance_work_mem = '256MB';

ALTER TABLE cdrs RENAME TO cdrs_unpartitioned;
ALTER TABLE cdrs_unpartitioned RENAME CONSTRAINT cdrs_pkey TO cdrs_unpartitioned_pkey;
ALTER INDEX IF EXISTS idx_cdrs_tenant RENAME TO idx_cdrs_unpartitioned_tenant;
ALTER INDEX IF EXISTS idx_cdrs_start_time RENAME TO idx_cdrs_unpartitioned_start_time;
ALTER INDEX IF EXISTS idx_cdrs_from_uri RENAME TO idx_cdrs_unpartitioned_from_uri;
ALTER INDEX IF EXISTS idx_cdrs_to_uri RENAME TO idx_cdrs_unpartitioned_to_uri;
ALTER INDEX IF EXISTS idx_cdrs_disposition RENAME TO idx_cdrs_unpartitioned_disposition;
ALTER INDEX IF EXISTS idx_cdrs_sip_call_id RENAME TO idx_cdrs_unpartitioned_sip_call_id;
ALTER INDEX IF EXISTS idx_cdrs_custom_data RENAME TO idx_cdrs_unpartitioned_custom_data;
ALTER INDEX IF EXISTS idx_cdrs_call_record RENAME TO idx_cdrs_unpartitioned_call_record;
ALTER INDEX IF EXISTS idx_cdrs_call_id RENAME TO idx_cdrs_unpartitioned_call_id;
ALTER INDEX IF EXISTS idx_cdrs_tenant_start RENAME TO idx_cdrs_unpartitioned_tenant_start;

CREATE TABLE cdrs (LIKE cdrs_unpartitioned INCLUDING DEFAULTS)
    PARTITION BY RANGE (start_time);
CREATE TABLE cdrs_default PARTITION OF cdrs DEFAULT;

-- A partition per month from the oldest record through three months ahead.
DO $$
DECLARE
    m DATE;
    last_month DATE := (date_trunc('month', now()) + INTERVAL '3 months')::date;
BEGIN
    SELECT date_trunc('month', LEAST(COALESCE(min(start_time), now()), now()))::date
      INTO m FROM cdrs_unpartitioned;
    WHILE m <= last_month LOOP
        EXECUTE format(
            'CREATE TABLE %I PARTITION OF cdrs FOR VALUES FROM (%L) TO (%L)',
            'cdrs_' || to_char(m, 'YYYY_MM'),
            m,
            (m + INTERVAL '1 month')::date
        );
        m := (m + INTERVAL '1 month')::date;
    END LOOP;
END $$;

INSERT INTO cdrs SELECT * FROM cdrs_unpartitioned;
DROP TABLE cdrs_unpartitioned;

ALTER TABLE cdrs ADD PRIMARY KEY (cdr_id, start_time);

-- Every listing, newest first, and its keyset pagination.
CREATE INDEX idx_cdrs_tenant_start ON cdrs (tenant_id, start_time DESC, cdr_id DESC);
-- A platform-wide listing.
CREATE INDEX idx_cdrs_start_time ON cdrs (start_time DESC, cdr_id DESC);
CREATE INDEX idx_cdrs_call_id ON cdrs (call_id);
CREATE UNIQUE INDEX idx_cdrs_call_record ON cdrs (call_id, start_time) WHERE kind = 'call';
CREATE INDEX idx_cdrs_sip_call_id ON cdrs (sip_call_id);
CREATE INDEX idx_cdrs_disposition ON cdrs (disposition);
CREATE INDEX idx_cdrs_custom_data ON cdrs USING GIN (custom_data);
-- idx_cdrs_from_uri and idx_cdrs_to_uri are not recreated: the URI filters
-- are substring matches, which a b-tree cannot serve; they run within a
-- tenant's time range on idx_cdrs_tenant_start.
