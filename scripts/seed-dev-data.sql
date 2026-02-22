-- FCP Development Seed Data
-- Apply AFTER migrations have run (i.e., after fcp-admin has started once)
--
-- Usage: docker exec -i fcp-postgres psql -U fcp -d fcp < scripts/seed-dev-data.sql

BEGIN;

-- ============================================================================
-- Tenant
-- ============================================================================
INSERT INTO tenants (id, name, description, status, contact_email, contact_phone,
    max_users, max_phone_numbers, max_concurrent_calls,
    created_at, updated_at, data)
VALUES (
    'default',
    'Acme Corp',
    'Development tenant for testing',
    'active',
    'admin@acme.example.com',
    '+15551001000',
    100, 50, 25,
    NOW(), NOW(),
    '{"plan": "enterprise", "region": "us-east"}'::jsonb
)
ON CONFLICT (id) DO NOTHING;

-- ============================================================================
-- Users (PK is tenant_id + username)
-- Password: password123 (bcrypt hash)
-- ============================================================================
INSERT INTO users (id, username, password_hash, display_name, tenant_id, enabled, role,
    email, extension, department, job_title, location)
VALUES
    ('usr-admin-001', 'admin', '$2y$12$Mw3LZOwLHJyexZSIS5oLXuRU9q2/iS43W/cSSx8BmWbkALVkyw7/S',
     'System Admin', 'default', true, 'super_admin',
     'admin@acme.example.com', NULL, 'IT', 'System Administrator', 'HQ'),

    ('usr-alice-001', 'alice', '$2y$12$Mw3LZOwLHJyexZSIS5oLXuRU9q2/iS43W/cSSx8BmWbkALVkyw7/S',
     'Alice Smith', 'default', true, 'end_user',
     'alice@acme.example.com', '100', 'Sales', 'Account Executive', 'HQ'),

    ('usr-bob-001', 'bob', '$2y$12$Mw3LZOwLHJyexZSIS5oLXuRU9q2/iS43W/cSSx8BmWbkALVkyw7/S',
     'Bob Jones', 'default', true, 'end_user',
     'bob@acme.example.com', '101', 'Engineering', 'Software Developer', 'Remote')
ON CONFLICT (tenant_id, username) DO NOTHING;

-- ============================================================================
-- Devices
-- ============================================================================
INSERT INTO devices (id, name, device_type, mac_address, model, firmware_version,
    tenant_id, owner_user_id, sip_uri, registration_state,
    max_lines, supports_video, supports_blf, codec_support)
VALUES
    ('dev-alice-desk', 'Alice Desk Phone', 'desk_phone',
     '00:15:65:AB:CD:01', 'Yealink T54W', '96.86.0.100',
     'default', 'usr-alice-001',
     'sip:alice@acme.example.com', 'unregistered',
     16, false, true,
     '["PCMU", "PCMA", "G722", "opus"]'::jsonb),

    ('dev-bob-desk', 'Bob Desk Phone', 'desk_phone',
     '64:16:7F:12:34:56', 'Polycom VVX450', '6.4.3.0000',
     'default', 'usr-bob-001',
     'sip:bob@acme.example.com', 'unregistered',
     12, true, true,
     '["PCMU", "PCMA", "G722", "opus"]'::jsonb)
ON CONFLICT (id) DO NOTHING;

-- ============================================================================
-- Phone Numbers (E.164)
-- ============================================================================
INSERT INTO phone_numbers (id, e164, country_code, npa, nxx, line, state,
    tenant_id, cnam, carrier, rate_center, acquired_at)
VALUES
    ('pn-main', '+15551001000', '1', '555', '100', '1000', 'active',
     'default', 'Acme Corp', 'Example Carrier', 'NEW YORK', NOW()),

    ('pn-alice', '+15551001001', '1', '555', '100', '1001', 'active',
     'default', 'Alice Smith', 'Example Carrier', 'NEW YORK', NOW()),

    ('pn-bob', '+15551001002', '1', '555', '100', '1002', 'active',
     'default', 'Bob Jones', 'Example Carrier', 'NEW YORK', NOW())
ON CONFLICT (id) DO NOTHING;

-- ============================================================================
-- Line Bindings (phone number -> user mappings)
-- ============================================================================
INSERT INTO line_bindings (id, phone_number_id, owner_user_id, binding_type,
    ring_strategy, tenant_id, no_answer_timeout_secs)
VALUES
    ('lb-alice', 'pn-alice', 'usr-alice-001', 'user',
     'ring_all', 'default', 30),

    ('lb-bob', 'pn-bob', 'usr-bob-001', 'user',
     'ring_all', 'default', 30)
ON CONFLICT (id) DO NOTHING;

-- ============================================================================
-- Extensions (short codes)
-- ============================================================================
INSERT INTO extensions (id, number, target_type, target_id, tenant_id, description)
VALUES
    ('ext-100', '100', 'user', 'usr-alice-001', 'default', 'Alice Smith direct'),
    ('ext-101', '101', 'user', 'usr-bob-001', 'default', 'Bob Jones direct')
ON CONFLICT (id) DO NOTHING;

-- ============================================================================
-- Voicemail Mailbox (for alice)
-- ============================================================================
INSERT INTO voicemail_mailboxes (id, tenant_id, user_id, line_binding_id,
    extension, pin, enabled, max_message_duration, max_messages,
    email_notification, email_address, transcription_enabled)
VALUES
    ('vm-alice', 'default', 'usr-alice-001', 'lb-alice',
     '100', '1234', true, 300, 100,
     true, 'alice@acme.example.com', true)
ON CONFLICT (id) DO NOTHING;

-- ============================================================================
-- Call Flow (simple auto-attendant)
-- ============================================================================
INSERT INTO flows (id, name, description, version, initial_node, nodes, variables,
    tenant_id, enabled)
VALUES (
    'flow-auto-attendant',
    'Main Auto-Attendant',
    'Greets callers and routes to departments',
    1,
    'greeting',
    '{
        "greeting": {
            "type": "play",
            "audio_url": "/audio/welcome.wav",
            "next": "menu"
        },
        "menu": {
            "type": "menu",
            "prompt_url": "/audio/menu-prompt.wav",
            "options": {
                "1": "sales",
                "2": "support",
                "0": "operator"
            },
            "timeout_seconds": 5,
            "max_retries": 3,
            "invalid_next": "menu",
            "timeout_next": "operator"
        },
        "sales": {
            "type": "transfer",
            "target": "100",
            "target_type": "extension"
        },
        "support": {
            "type": "transfer",
            "target": "101",
            "target_type": "extension"
        },
        "operator": {
            "type": "transfer",
            "target": "100",
            "target_type": "extension"
        }
    }'::jsonb,
    '{}'::jsonb,
    'default',
    true
)
ON CONFLICT (id) DO NOTHING;

-- ============================================================================
-- CDRs (sample completed calls)
-- ============================================================================
INSERT INTO cdrs (call_id, tenant_id, sip_call_id,
    from_uri, from_display_name, to_uri, to_display_name,
    start_time, answer_time, end_time,
    setup_duration_ms, call_duration_ms, total_duration_ms,
    disposition, hangup_cause, hangup_party,
    sip_status_code, sip_reason,
    media_anchored, codec_a_leg, codec_b_leg)
VALUES
    ('cdr-001', 'default', 'abc123@fcp.local',
     'sip:alice@acme.example.com', 'Alice Smith',
     'sip:bob@acme.example.com', 'Bob Jones',
     NOW() - INTERVAL '2 hours', NOW() - INTERVAL '2 hours' + INTERVAL '3 seconds',
     NOW() - INTERVAL '2 hours' + INTERVAL '183 seconds',
     3000, 180000, 183000,
     'answered', 'normal_clearing', 'caller',
     200, 'OK',
     true, 'PCMU', 'PCMU'),

    ('cdr-002', 'default', 'def456@fcp.local',
     'sip:+15559990001@carrier.example.com', 'External Caller',
     'sip:alice@acme.example.com', 'Alice Smith',
     NOW() - INTERVAL '30 minutes', NOW() - INTERVAL '30 minutes' + INTERVAL '5 seconds',
     NOW() - INTERVAL '30 minutes' + INTERVAL '65 seconds',
     5000, 60000, 65000,
     'answered', 'normal_clearing', 'callee',
     200, 'OK',
     true, 'G722', 'G722')
ON CONFLICT (call_id) DO NOTHING;

COMMIT;
