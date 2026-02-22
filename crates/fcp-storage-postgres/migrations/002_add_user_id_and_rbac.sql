-- Migration 004: Add user ID, RBAC fields, and enterprise user fields
-- Version: 0.4.0
-- Note: Some columns may already exist from 001_initial_schema.sql; using IF NOT EXISTS

-- Add id column (unique user identifier)
ALTER TABLE users ADD COLUMN IF NOT EXISTS id TEXT;

-- Add RBAC fields
ALTER TABLE users ADD COLUMN IF NOT EXISTS role TEXT NOT NULL DEFAULT 'end_user';
ALTER TABLE users ADD COLUMN IF NOT EXISTS department_id TEXT;

-- Add enterprise user fields
ALTER TABLE users ADD COLUMN IF NOT EXISTS email TEXT;
ALTER TABLE users ADD COLUMN IF NOT EXISTS password_last_reset TIMESTAMPTZ;
ALTER TABLE users ADD COLUMN IF NOT EXISTS phone_number TEXT;
ALTER TABLE users ADD COLUMN IF NOT EXISTS mobile_number TEXT;
ALTER TABLE users ADD COLUMN IF NOT EXISTS pin TEXT;
ALTER TABLE users ADD COLUMN IF NOT EXISTS extension TEXT;
ALTER TABLE users ADD COLUMN IF NOT EXISTS department TEXT;
ALTER TABLE users ADD COLUMN IF NOT EXISTS job_title TEXT;
ALTER TABLE users ADD COLUMN IF NOT EXISTS location TEXT;
ALTER TABLE users ADD COLUMN IF NOT EXISTS language TEXT NOT NULL DEFAULT 'en-US';
ALTER TABLE users ADD COLUMN IF NOT EXISTS timezone TEXT NOT NULL DEFAULT 'UTC';
ALTER TABLE users ADD COLUMN IF NOT EXISTS account_status TEXT NOT NULL DEFAULT 'active';
ALTER TABLE users ADD COLUMN IF NOT EXISTS last_login TIMESTAMPTZ;
ALTER TABLE users ADD COLUMN IF NOT EXISTS emergency_contact_json JSONB;
ALTER TABLE users ADD COLUMN IF NOT EXISTS primary_phone_number_id TEXT;
ALTER TABLE users ADD COLUMN IF NOT EXISTS voicemail_box_id TEXT;

-- Create index on id column
CREATE INDEX IF NOT EXISTS idx_users_id ON users(id);

-- Create index on role for permission queries
CREATE INDEX IF NOT EXISTS idx_users_role ON users(role);
