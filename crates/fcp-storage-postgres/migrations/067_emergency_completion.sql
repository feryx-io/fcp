-- E911 block 2 phase 1 (docs/E911_BLOCK_2.md §4): an emergency call's
-- record is written when the call is placed and completed when it ends —
-- the trunk that answered, the final SIP status, when it was answered and
-- ended, and how the location travelled.

ALTER TABLE emergency_calls
    ADD COLUMN IF NOT EXISTS sip_status INTEGER,
    ADD COLUMN IF NOT EXISTS answered_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS ended_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS delivery TEXT;
