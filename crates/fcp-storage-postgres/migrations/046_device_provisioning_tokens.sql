-- Per-device provisioning token hashes and global MAC uniqueness.
--
-- Auto-provisioning is addressed by MAC, so a normalized MAC must identify at
-- most one device across the deployment. Device-specific token hashes prevent a
-- leaked deployment provisioning token from authorizing every known MAC.

ALTER TABLE devices ADD COLUMN IF NOT EXISTS provisioning_token_hash TEXT;

-- Abort loudly if existing rows would violate the unique index. Operators must
-- resolve duplicates before this migration can succeed. Listing the offending
-- MACs in the error message makes that cleanup actionable without a separate
-- diagnostic query.
DO $$
DECLARE
    duplicate_macs TEXT;
BEGIN
    SELECT STRING_AGG(normalized_mac, ', ')
    INTO duplicate_macs
    FROM (
        SELECT LOWER(REPLACE(REPLACE(REPLACE(mac_address, ':', ''), '-', ''), '.', '')) AS normalized_mac
        FROM devices
        WHERE mac_address IS NOT NULL AND BTRIM(mac_address) <> ''
        GROUP BY 1
        HAVING COUNT(*) > 1
    ) d;

    IF duplicate_macs IS NOT NULL THEN
        RAISE EXCEPTION
            'Migration 046 cannot apply: duplicate MAC addresses exist across devices (%). Deduplicate before running this migration.',
            duplicate_macs;
    END IF;
END $$;

CREATE UNIQUE INDEX IF NOT EXISTS idx_devices_normalized_mac_unique
ON devices (LOWER(REPLACE(REPLACE(REPLACE(mac_address, ':', ''), '-', ''), '.', '')))
WHERE mac_address IS NOT NULL AND BTRIM(mac_address) <> '';
