-- A trunk that takes numbers in E.164 (docs/DEEP_TESTING.md D8): the
-- dialled number as the tenant's country reads it, before the trunk's
-- digit plan. Off for every existing trunk, which keeps sending numbers as
-- they were dialled.
ALTER TABLE trunks ADD COLUMN IF NOT EXISTS send_e164 BOOLEAN NOT NULL DEFAULT false;
