-- Consent on every recording (docs/COMPLIANCE_FOLLOWUPS.md §8): what the
-- tenant does when the announcement cannot be played, the continuing notice
-- tone, and what a call's parties were told, on its record.

ALTER TABLE compliance_policies
    ADD COLUMN IF NOT EXISTS recording_consent_on_failure TEXT NOT NULL DEFAULT 'record';
ALTER TABLE compliance_policies
    ADD COLUMN IF NOT EXISTS recording_notice_interval_secs INTEGER;

ALTER TABLE cdrs ADD COLUMN IF NOT EXISTS recording_consent TEXT;
