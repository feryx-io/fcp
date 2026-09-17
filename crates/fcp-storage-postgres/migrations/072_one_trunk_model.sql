-- Tenant trunks phase 1 (docs/TENANT_TRUNKS.md §4): one trunk model.
--
-- A trunk a tenant manages and a trunk a node reads from its own [[trunks]]
-- are now the same type, so the stored trunk gains the settings only a TOML
-- trunk could express. Nothing reads them on a stored trunk until tenant
-- trunks route, but the model is no longer lossy.

ALTER TABLE trunks
    -- The named prefixes of an egress digit plan; strip_digits stays as the
    -- "take off this many" form.
    ADD COLUMN IF NOT EXISTS strip_prefixes TEXT[] NOT NULL DEFAULT ARRAY[]::TEXT[],
    -- SRTP crypto suites (empty = every supported suite).
    ADD COLUMN IF NOT EXISTS srtp_suites TEXT[] NOT NULL DEFAULT ARRAY[]::TEXT[],
    -- Whether the media pipeline may transcode for this trunk.
    ADD COLUMN IF NOT EXISTS allow_transcoding BOOLEAN NOT NULL DEFAULT TRUE,
    -- STIR/SHAKEN per trunk: 'disabled' | 'log_only' | 'enforce'.
    ADD COLUMN IF NOT EXISTS stir_verify_mode TEXT NOT NULL DEFAULT 'disabled',
    -- 'disabled' | 'enabled'.
    ADD COLUMN IF NOT EXISTS stir_sign_mode TEXT NOT NULL DEFAULT 'disabled',
    -- 'A' | 'B' | 'C' (RFC 8588 §5.2.1).
    ADD COLUMN IF NOT EXISTS stir_attestation TEXT NOT NULL DEFAULT 'A',
    ADD COLUMN IF NOT EXISTS stir_max_iat_age_secs BIGINT NOT NULL DEFAULT 60,
    -- How an emergency call's location travels: 'elin', 'pidf_lo', or both.
    ADD COLUMN IF NOT EXISTS emergency_location TEXT[] NOT NULL DEFAULT ARRAY['elin']::TEXT[];
