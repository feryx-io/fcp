-- Migration 002: Add enterprise-grade user fields
-- Version: 0.2.0

-- Add new columns to users table
ALTER TABLE users ADD COLUMN email TEXT;
ALTER TABLE users ADD COLUMN password_last_reset INTEGER;
ALTER TABLE users ADD COLUMN phone_number TEXT;
ALTER TABLE users ADD COLUMN mobile_number TEXT;
ALTER TABLE users ADD COLUMN pin TEXT;
ALTER TABLE users ADD COLUMN extension TEXT;
ALTER TABLE users ADD COLUMN department TEXT;
ALTER TABLE users ADD COLUMN job_title TEXT;
ALTER TABLE users ADD COLUMN location TEXT;
ALTER TABLE users ADD COLUMN language TEXT DEFAULT 'en-US';
ALTER TABLE users ADD COLUMN timezone TEXT DEFAULT 'UTC';
ALTER TABLE users ADD COLUMN account_status TEXT DEFAULT 'active';
ALTER TABLE users ADD COLUMN last_login INTEGER;
ALTER TABLE users ADD COLUMN emergency_contact_name TEXT;
ALTER TABLE users ADD COLUMN emergency_contact_phone TEXT;

-- Create indexes for frequently queried fields
CREATE INDEX IF NOT EXISTS idx_users_email ON users(email);
CREATE INDEX IF NOT EXISTS idx_users_extension ON users(extension);
CREATE INDEX IF NOT EXISTS idx_users_department ON users(department);
CREATE INDEX IF NOT EXISTS idx_users_account_status ON users(account_status);
