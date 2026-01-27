-- Migration 007: Add user ID and RBAC fields
-- Version: 0.7.0
-- SQLite equivalent of PostgreSQL migration 004_add_user_id_and_rbac.sql

-- Add id column (unique user identifier, persisted across reads)
ALTER TABLE users ADD COLUMN id TEXT;

-- Add RBAC fields
ALTER TABLE users ADD COLUMN role TEXT NOT NULL DEFAULT 'end_user';
ALTER TABLE users ADD COLUMN department_id TEXT;

-- Create index on id column
CREATE INDEX IF NOT EXISTS idx_users_id ON users(id);

-- Create index on role for permission queries
CREATE INDEX IF NOT EXISTS idx_users_role ON users(role);
