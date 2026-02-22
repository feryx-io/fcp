-- Add Path header support for registrations (RFC 3327)
-- Version: 0.14.31

-- Add path column to registrations table
-- Stores serialized Path header for routing through proxies
ALTER TABLE registrations ADD COLUMN IF NOT EXISTS path TEXT;
