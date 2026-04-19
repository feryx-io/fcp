-- Optional plaintext SIP password for vendor auto-provisioning templates.
--
-- SIP authentication continues to use sip_ha1_hash. This column is only
-- populated when an administrator explicitly stores the SIP password needed by
-- phones that cannot provision from HA1/precomputed digest material.

ALTER TABLE users ADD COLUMN IF NOT EXISTS sip_provisioning_password TEXT;
