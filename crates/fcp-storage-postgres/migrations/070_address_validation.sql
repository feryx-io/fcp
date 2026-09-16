-- E911 block 2 phase 4 (docs/E911_BLOCK_2.md §7): validation.
--
-- A site's address is validated like a phone number's: the validator's
-- answer is kept with the site, so readiness can say the address has been
-- checked and by whom. Editing any part of the address clears it again.
-- Phone number and device addresses carry the same marks inside their
-- JSON, which needs no column.

ALTER TABLE locations
    ADD COLUMN IF NOT EXISTS e911_validated_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS e911_validated_by TEXT,
    ADD COLUMN IF NOT EXISTS e911_validation_reference TEXT;
