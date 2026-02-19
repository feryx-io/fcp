-- Add advanced authorization fields from P2 (event-based, time-window, pattern-based auth)
ALTER TABLE presence_auth_policies ADD COLUMN event_allow_json TEXT NOT NULL DEFAULT '{}';
ALTER TABLE presence_auth_policies ADD COLUMN event_block_json TEXT NOT NULL DEFAULT '{}';
ALTER TABLE presence_auth_policies ADD COLUMN time_windows_json TEXT NOT NULL DEFAULT '[]';
ALTER TABLE presence_auth_policies ADD COLUMN allow_patterns_json TEXT NOT NULL DEFAULT '[]';
ALTER TABLE presence_auth_policies ADD COLUMN block_patterns_json TEXT NOT NULL DEFAULT '[]';
