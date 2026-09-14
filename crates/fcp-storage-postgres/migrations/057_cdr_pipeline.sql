-- The CDR pipeline, phase 1 (docs/CDR_PIPELINE.md §5): the record's own
-- identity and kind, the columns the node fills at hangup, and what the
-- rating stamped. Existing rows get a deterministic cdr_id from their
-- call_id, so running this again would mint the same ids.

ALTER TABLE cdrs ADD COLUMN IF NOT EXISTS cdr_id UUID;
UPDATE cdrs SET cdr_id = md5(call_id)::uuid WHERE cdr_id IS NULL;
ALTER TABLE cdrs ALTER COLUMN cdr_id SET NOT NULL;
ALTER TABLE cdrs ALTER COLUMN cdr_id SET DEFAULT gen_random_uuid();

ALTER TABLE cdrs ADD COLUMN IF NOT EXISTS kind TEXT NOT NULL DEFAULT 'call';
-- The queue handler's records were the only non-call records before this.
UPDATE cdrs SET kind = 'queue' WHERE kind = 'call' AND to_uri LIKE 'queue:%';
ALTER TABLE cdrs ADD COLUMN IF NOT EXISTS schema_version SMALLINT NOT NULL DEFAULT 1;

ALTER TABLE cdrs ADD COLUMN IF NOT EXISTS direction TEXT;
ALTER TABLE cdrs ADD COLUMN IF NOT EXISTS node_id TEXT;
ALTER TABLE cdrs ADD COLUMN IF NOT EXISTS region TEXT;
ALTER TABLE cdrs ADD COLUMN IF NOT EXISTS correlation_id TEXT;
ALTER TABLE cdrs ADD COLUMN IF NOT EXISTS trunk_name TEXT;
-- The B2BUA stored the trunk's name in gateway_id until now.
UPDATE cdrs SET trunk_name = gateway_id WHERE trunk_name IS NULL AND gateway_id IS NOT NULL;

ALTER TABLE cdrs ADD COLUMN IF NOT EXISTS rtp_packets_lost BIGINT;
ALTER TABLE cdrs ADD COLUMN IF NOT EXISTS rtp_jitter_ms DOUBLE PRECISION;

ALTER TABLE cdrs ADD COLUMN IF NOT EXISTS rate_plan_id TEXT;
ALTER TABLE cdrs ADD COLUMN IF NOT EXISTS matched_prefix TEXT;
ALTER TABLE cdrs ADD COLUMN IF NOT EXISTS billable_seconds INTEGER;
ALTER TABLE cdrs ADD COLUMN IF NOT EXISTS rated_at TIMESTAMPTZ;
ALTER TABLE cdrs ADD COLUMN IF NOT EXISTS created_at TIMESTAMPTZ NOT NULL DEFAULT now();

-- The record's identity is cdr_id; call_id is unique among call records
-- only, since the other kinds sit beside the call's own record.
ALTER TABLE cdrs DROP CONSTRAINT IF EXISTS cdrs_pkey;
ALTER TABLE cdrs ADD PRIMARY KEY (cdr_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_cdrs_call_record ON cdrs (call_id) WHERE kind = 'call';
CREATE INDEX IF NOT EXISTS idx_cdrs_call_id ON cdrs (call_id);

-- The shape of every listing: a tenant's records, newest first.
CREATE INDEX IF NOT EXISTS idx_cdrs_tenant_start ON cdrs (tenant_id, start_time DESC);
