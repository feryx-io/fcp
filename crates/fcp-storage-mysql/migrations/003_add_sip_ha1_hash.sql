-- Migration 003: Add separate digest credentials (Cisco-style security)
-- Version: 0.3.0

-- Add digest_password column to users table
-- This is an optional separate password for SIP authentication (Cisco CUCM style)
-- If NULL, the main password is used for SIP authentication
-- Stored as bcrypt hash just like the main password
ALTER TABLE users ADD COLUMN digest_password TEXT;

-- Add sip_ha1_hash column to users table
-- This stores the pre-computed HA1 hash (MD5(username:realm:password))
-- Computed from digest_password (if set) or password (if digest_password is NULL)
-- Used for SIP Digest authentication per RFC 7616/7617
ALTER TABLE users ADD COLUMN sip_ha1_hash TEXT;
