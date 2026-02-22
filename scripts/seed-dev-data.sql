-- FCP Development Seed Data
-- Apply AFTER migrations have run (i.e., after fcp-admin has started once)
--
-- Usage: docker exec -i fcp-postgres psql -U fcp -d fcp < scripts/seed-dev-data.sql
--
-- NOTE: fcp-admin uses TenantId::nil() as default_tenant_id, which is
-- "tenant-00000000-0000-0000-0000-000000000000". All seed data must use this
-- tenant ID to appear in the dashboard and API.
--
-- All IDs must be valid UUIDs (the Rust types parse the UUID after stripping prefix).

BEGIN;

-- Use the nil-UUID tenant that fcp-admin defaults to
-- The Display format is "tenant-00000000-0000-0000-0000-000000000000"
-- but the DB stores just the text as passed.

-- ============================================================================
-- Tenant
-- ============================================================================
INSERT INTO tenants (id, name, description, status, contact_email, contact_phone,
    max_users, max_phone_numbers, max_concurrent_calls,
    created_at, updated_at, data)
VALUES (
    'tenant-00000000-0000-0000-0000-000000000000',
    'Acme Corp',
    'Development tenant for testing',
    'active',
    'admin@acme.example.com',
    '+15551001000',
    100, 50, 25,
    NOW(), NOW(),
    '{"plan": "enterprise", "region": "us-east"}'::jsonb
)
ON CONFLICT (id) DO UPDATE SET
    name = EXCLUDED.name,
    description = EXCLUDED.description,
    contact_email = EXCLUDED.contact_email,
    contact_phone = EXCLUDED.contact_phone;

-- ============================================================================
-- Users (PK is tenant_id + username)
-- Password: password123 (bcrypt hash)
-- IDs must be valid UUIDs (UserId strips "user-" prefix then parses UUID)
-- ============================================================================
INSERT INTO users (id, username, password_hash, display_name, tenant_id, enabled, role,
    email, extension, department, job_title, location,
    media_preferences, emergency_contact_json)
VALUES
    ('user-00000000-0000-0000-0000-000000000001', 'admin',
     '$2y$12$Mw3LZOwLHJyexZSIS5oLXuRU9q2/iS43W/cSSx8BmWbkALVkyw7/S',
     'System Admin', 'tenant-00000000-0000-0000-0000-000000000000', true, 'super_admin',
     'admin@acme.example.com', NULL, 'IT', 'System Administrator', 'HQ',
     '{}'::jsonb, '{}'::jsonb),

    ('user-00000000-0000-0000-0000-000000000002', 'alice',
     '$2y$12$Mw3LZOwLHJyexZSIS5oLXuRU9q2/iS43W/cSSx8BmWbkALVkyw7/S',
     'Alice Smith', 'tenant-00000000-0000-0000-0000-000000000000', true, 'end_user',
     'alice@acme.example.com', '100', 'Sales', 'Account Executive', 'HQ',
     '{}'::jsonb, '{}'::jsonb),

    ('user-00000000-0000-0000-0000-000000000003', 'bob',
     '$2y$12$Mw3LZOwLHJyexZSIS5oLXuRU9q2/iS43W/cSSx8BmWbkALVkyw7/S',
     'Bob Jones', 'tenant-00000000-0000-0000-0000-000000000000', true, 'end_user',
     'bob@acme.example.com', '101', 'Engineering', 'Software Developer', 'Remote',
     '{}'::jsonb, '{}'::jsonb)
ON CONFLICT (tenant_id, username) DO UPDATE SET
    id = EXCLUDED.id,
    display_name = EXCLUDED.display_name,
    email = EXCLUDED.email,
    extension = EXCLUDED.extension,
    department = EXCLUDED.department,
    job_title = EXCLUDED.job_title,
    location = EXCLUDED.location,
    media_preferences = EXCLUDED.media_preferences,
    emergency_contact_json = EXCLUDED.emergency_contact_json;

-- ============================================================================
-- Devices (IDs must be valid UUIDs with "dev-" prefix)
-- ============================================================================
INSERT INTO devices (id, name, device_type, mac_address, model, firmware_version,
    tenant_id, owner_user_id, sip_uri, registration_state,
    max_lines, supports_video, supports_blf, codec_support)
VALUES
    ('dev-00000000-0000-0000-0000-000000000001', 'Alice Desk Phone', 'desk_phone',
     '00:15:65:AB:CD:01', 'Yealink T54W', '96.86.0.100',
     'tenant-00000000-0000-0000-0000-000000000000', 'user-00000000-0000-0000-0000-000000000002',
     'sip:alice@acme.example.com', 'unregistered',
     16, false, true,
     '["PCMU", "PCMA", "G722", "opus"]'::jsonb),

    ('dev-00000000-0000-0000-0000-000000000002', 'Bob Desk Phone', 'desk_phone',
     '64:16:7F:12:34:56', 'Polycom VVX450', '6.4.3.0000',
     'tenant-00000000-0000-0000-0000-000000000000', 'user-00000000-0000-0000-0000-000000000003',
     'sip:bob@acme.example.com', 'unregistered',
     12, true, true,
     '["PCMU", "PCMA", "G722", "opus"]'::jsonb)
ON CONFLICT (id) DO UPDATE SET
    name = EXCLUDED.name,
    model = EXCLUDED.model,
    firmware_version = EXCLUDED.firmware_version,
    owner_user_id = EXCLUDED.owner_user_id;

-- ============================================================================
-- Phone Numbers (E.164)
-- ============================================================================
INSERT INTO phone_numbers (id, e164, country_code, npa, nxx, line, state,
    tenant_id, cnam, carrier, rate_center, acquired_at)
VALUES
    ('num-00000000-0000-0000-0000-000000000001', '+15551001000', '1', '555', '100', '1000', 'active',
     'tenant-00000000-0000-0000-0000-000000000000', 'Acme Corp', 'Example Carrier', 'NEW YORK', NOW()),

    ('num-00000000-0000-0000-0000-000000000002', '+15551001001', '1', '555', '100', '1001', 'active',
     'tenant-00000000-0000-0000-0000-000000000000', 'Alice Smith', 'Example Carrier', 'NEW YORK', NOW()),

    ('num-00000000-0000-0000-0000-000000000003', '+15551001002', '1', '555', '100', '1002', 'active',
     'tenant-00000000-0000-0000-0000-000000000000', 'Bob Jones', 'Example Carrier', 'NEW YORK', NOW())
ON CONFLICT (id) DO UPDATE SET
    cnam = EXCLUDED.cnam,
    state = EXCLUDED.state;

-- ============================================================================
-- Line Bindings (phone number -> user mappings)
-- ============================================================================
INSERT INTO line_bindings (id, phone_number_id, owner_user_id, binding_type,
    ring_strategy, tenant_id, no_answer_timeout_secs)
VALUES
    ('bind-00000000-0000-0000-0000-000000000001',
     'num-00000000-0000-0000-0000-000000000002', 'user-00000000-0000-0000-0000-000000000002',
     'user', 'ring_all', 'tenant-00000000-0000-0000-0000-000000000000', 30),

    ('bind-00000000-0000-0000-0000-000000000002',
     'num-00000000-0000-0000-0000-000000000003', 'user-00000000-0000-0000-0000-000000000003',
     'user', 'ring_all', 'tenant-00000000-0000-0000-0000-000000000000', 30)
ON CONFLICT (id) DO NOTHING;

-- ============================================================================
-- Extensions (short codes)
-- ============================================================================
INSERT INTO extensions (id, number, target_type, target_id, tenant_id, description)
VALUES
    ('ext-00000000-0000-0000-0000-000000000001', '100', 'user',
     'user-00000000-0000-0000-0000-000000000002',
     'tenant-00000000-0000-0000-0000-000000000000', 'Alice Smith direct'),
    ('ext-00000000-0000-0000-0000-000000000002', '101', 'user',
     'user-00000000-0000-0000-0000-000000000003',
     'tenant-00000000-0000-0000-0000-000000000000', 'Bob Jones direct')
ON CONFLICT (id) DO NOTHING;

-- ============================================================================
-- Voicemail Mailbox (for alice)
-- ============================================================================
INSERT INTO voicemail_mailboxes (id, tenant_id, user_id, line_binding_id,
    extension, pin, enabled, max_message_duration, max_messages,
    email_notification, email_address, transcription_enabled)
VALUES
    ('vm-00000000-0000-0000-0000-000000000001',
     'tenant-00000000-0000-0000-0000-000000000000',
     'user-00000000-0000-0000-0000-000000000002',
     'bind-00000000-0000-0000-0000-000000000001',
     '100', '1234', true, 300, 100,
     true, 'alice@acme.example.com', true)
ON CONFLICT (id) DO NOTHING;

-- ============================================================================
-- Call Flow (simple auto-attendant)
-- ============================================================================
INSERT INTO flows (id, name, description, version, initial_node, nodes, variables,
    tenant_id, enabled)
VALUES (
    'flow-00000000-0000-0000-0000-000000000001',
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
    'tenant-00000000-0000-0000-0000-000000000000',
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
    ('call-00000000-0000-0000-0000-000000000001',
     'tenant-00000000-0000-0000-0000-000000000000', 'abc123@fcp.local',
     'sip:alice@acme.example.com', 'Alice Smith',
     'sip:bob@acme.example.com', 'Bob Jones',
     NOW() - INTERVAL '2 hours', NOW() - INTERVAL '2 hours' + INTERVAL '3 seconds',
     NOW() - INTERVAL '2 hours' + INTERVAL '183 seconds',
     3000, 180000, 183000,
     'answered', 'normal_clearing', 'caller',
     200, 'OK',
     true, 'PCMU', 'PCMU'),

    ('call-00000000-0000-0000-0000-000000000002',
     'tenant-00000000-0000-0000-0000-000000000000', 'def456@fcp.local',
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
