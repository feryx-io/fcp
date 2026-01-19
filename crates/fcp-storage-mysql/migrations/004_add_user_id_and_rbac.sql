-- Migration 004: Add user ID, RBAC fields, and enterprise user fields
-- Version: 0.4.0

-- Add id column (unique user identifier)
ALTER TABLE users ADD COLUMN id VARCHAR(255);

-- Add RBAC fields
ALTER TABLE users ADD COLUMN role VARCHAR(50) NOT NULL DEFAULT 'end_user';
ALTER TABLE users ADD COLUMN department_id VARCHAR(255);

-- Add enterprise user fields (if not already added)
ALTER TABLE users ADD COLUMN email VARCHAR(255);
ALTER TABLE users ADD COLUMN password_last_reset DATETIME(6);
ALTER TABLE users ADD COLUMN phone_number VARCHAR(50);
ALTER TABLE users ADD COLUMN mobile_number VARCHAR(50);
ALTER TABLE users ADD COLUMN pin VARCHAR(255);
ALTER TABLE users ADD COLUMN extension VARCHAR(50);
ALTER TABLE users ADD COLUMN department VARCHAR(255);
ALTER TABLE users ADD COLUMN job_title VARCHAR(255);
ALTER TABLE users ADD COLUMN location VARCHAR(255);
ALTER TABLE users ADD COLUMN language VARCHAR(50) NOT NULL DEFAULT 'en-US';
ALTER TABLE users ADD COLUMN timezone VARCHAR(100) NOT NULL DEFAULT 'UTC';
ALTER TABLE users ADD COLUMN account_status VARCHAR(50) NOT NULL DEFAULT 'active';
ALTER TABLE users ADD COLUMN last_login DATETIME(6);
ALTER TABLE users ADD COLUMN emergency_contact_name VARCHAR(255);
ALTER TABLE users ADD COLUMN emergency_contact_phone VARCHAR(50);
ALTER TABLE users ADD COLUMN primary_phone_number_id VARCHAR(255);
ALTER TABLE users ADD COLUMN voicemail_box_id VARCHAR(255);

-- Create index on id column
CREATE INDEX idx_users_id ON users(id);

-- Create index on role for permission queries
CREATE INDEX idx_users_role ON users(role);
