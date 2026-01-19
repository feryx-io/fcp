-- Migration 004: Add user ID, RBAC fields, and enterprise user fields
-- Version: 0.4.0

-- Add id column (unique user identifier)
ALTER TABLE users ADD COLUMN id TEXT;

-- Add RBAC fields
ALTER TABLE users ADD COLUMN role TEXT NOT NULL DEFAULT 'end_user';
ALTER TABLE users ADD COLUMN department_id TEXT;

-- Add enterprise user fields (if not already added)
ALTER TABLE users ADD COLUMN email TEXT;
ALTER TABLE users ADD COLUMN password_last_reset TIMESTAMPTZ;
ALTER TABLE users ADD COLUMN phone_number TEXT;
ALTER TABLE users ADD COLUMN mobile_number TEXT;
ALTER TABLE users ADD COLUMN pin TEXT;
ALTER TABLE users ADD COLUMN extension TEXT;
ALTER TABLE users ADD COLUMN department TEXT;
ALTER TABLE users ADD COLUMN job_title TEXT;
ALTER TABLE users ADD COLUMN location TEXT;
ALTER TABLE users ADD COLUMN language TEXT NOT NULL DEFAULT 'en-US';
ALTER TABLE users ADD COLUMN timezone TEXT NOT NULL DEFAULT 'UTC';
ALTER TABLE users ADD COLUMN account_status TEXT NOT NULL DEFAULT 'active';
ALTER TABLE users ADD COLUMN last_login TIMESTAMPTZ;
ALTER TABLE users ADD COLUMN emergency_contact_name TEXT;
ALTER TABLE users ADD COLUMN emergency_contact_phone TEXT;
ALTER TABLE users ADD COLUMN primary_phone_number_id TEXT;
ALTER TABLE users ADD COLUMN voicemail_box_id TEXT;

-- Create index on id column
CREATE INDEX IF NOT EXISTS idx_users_id ON users(id);

-- Create index on role for permission queries
CREATE INDEX IF NOT EXISTS idx_users_role ON users(role);
