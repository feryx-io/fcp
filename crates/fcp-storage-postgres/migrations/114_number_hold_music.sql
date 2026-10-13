-- Music on hold per number (docs/ENTERPRISE_CALLING.md §3.3): the music a
-- caller to this number hears on hold, unless the holder chose their own —
-- a prompt of the tenant by id, or `none` for silence. NULL: the tenant's
-- hold music, else the node's built-in.
ALTER TABLE line_bindings ADD COLUMN IF NOT EXISTS hold_music TEXT;
