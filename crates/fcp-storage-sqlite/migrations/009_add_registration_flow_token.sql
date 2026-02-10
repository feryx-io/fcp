-- Add flow_token for RFC 5626 Outbound support
-- Version: 0.14.31

-- Add flow_token column to registrations table
-- Identifies the TLS/TCP connection for reusing inbound connections
-- when routing requests to NAT'd clients.
ALTER TABLE registrations ADD COLUMN flow_token TEXT;
