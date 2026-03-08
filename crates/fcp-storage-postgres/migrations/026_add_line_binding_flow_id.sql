-- Add flow_id column to line_bindings for IVR routing
ALTER TABLE line_bindings ADD COLUMN IF NOT EXISTS flow_id TEXT;
