-- Who answers an emergency test call (933, docs/E911.md "Test calls"):
-- 'local', the call manager reading back what a real call would carry, or
-- 'carrier', the call sent to the emergency trunks for the carrier's own
-- test service. Every tenant starts local; a tenant that relied on its
-- carrier's readback sets 'carrier'.
ALTER TABLE emergency_configs
    ADD COLUMN IF NOT EXISTS test_call_mode TEXT NOT NULL DEFAULT 'local'
    CHECK (test_call_mode IN ('local', 'carrier'));
