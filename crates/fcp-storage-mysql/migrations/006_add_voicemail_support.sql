-- Voicemail support schema
-- Version: 0.3.0
-- This migration adds tables for voicemail mailboxes, messages, and greetings

-- Voicemail mailboxes table
CREATE TABLE IF NOT EXISTS voicemail_mailboxes (
    id VARCHAR(255) PRIMARY KEY,
    tenant_id VARCHAR(255) NOT NULL,
    user_id VARCHAR(255) NOT NULL,
    line_binding_id VARCHAR(255) NOT NULL,
    extension VARCHAR(20) NOT NULL,
    pin VARCHAR(255) NOT NULL, -- hashed PIN for mailbox access
    enabled BOOLEAN NOT NULL DEFAULT TRUE,

    -- Limits
    max_message_duration INT NOT NULL DEFAULT 300, -- 5 minutes
    max_messages INT NOT NULL DEFAULT 100,

    -- Notifications
    email_notification BOOLEAN NOT NULL DEFAULT FALSE,
    email_address VARCHAR(255),
    attach_audio BOOLEAN NOT NULL DEFAULT FALSE,
    include_transcription BOOLEAN NOT NULL DEFAULT FALSE,
    transcription_enabled BOOLEAN NOT NULL DEFAULT FALSE,

    -- Audit
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP,

    -- Constraints
    UNIQUE(tenant_id, extension)
    -- Foreign keys temporarily disabled for Phase 1
    -- Will be re-enabled in Phase 2 with proper migration strategy
    -- FOREIGN KEY (tenant_id) REFERENCES tenants(id) ON DELETE CASCADE,
    -- FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
    -- FOREIGN KEY (line_binding_id) REFERENCES line_bindings(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_voicemail_mailboxes_tenant ON voicemail_mailboxes(tenant_id);
CREATE INDEX IF NOT EXISTS idx_voicemail_mailboxes_user ON voicemail_mailboxes(user_id);
CREATE INDEX IF NOT EXISTS idx_voicemail_mailboxes_line_binding ON voicemail_mailboxes(line_binding_id);
CREATE INDEX IF NOT EXISTS idx_voicemail_mailboxes_extension ON voicemail_mailboxes(tenant_id, extension);

-- Voicemail messages table
CREATE TABLE IF NOT EXISTS voicemail_messages (
    id VARCHAR(255) PRIMARY KEY,
    mailbox_id VARCHAR(255) NOT NULL,
    caller VARCHAR(50) NOT NULL, -- E.164 formatted phone number
    caller_name VARCHAR(255), -- Caller ID name if available
    status VARCHAR(20) NOT NULL CHECK(status IN ('new', 'heard', 'saved', 'deleted')),
    priority VARCHAR(20) NOT NULL CHECK(priority IN ('normal', 'urgent')),
    duration INT NOT NULL, -- Duration in seconds
    recording_path TEXT NOT NULL, -- Path to audio file
    transcription TEXT, -- Optional transcription text

    -- Timestamps
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    accessed_at TIMESTAMP NULL -- When message was last played/accessed

    -- Foreign keys temporarily disabled for Phase 1
    -- FOREIGN KEY (mailbox_id) REFERENCES voicemail_mailboxes(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_voicemail_messages_mailbox ON voicemail_messages(mailbox_id);
CREATE INDEX IF NOT EXISTS idx_voicemail_messages_status ON voicemail_messages(mailbox_id, status);
CREATE INDEX IF NOT EXISTS idx_voicemail_messages_created ON voicemail_messages(mailbox_id, created_at DESC);

-- Voicemail greetings table
CREATE TABLE IF NOT EXISTS voicemail_greetings (
    id BIGINT AUTO_INCREMENT PRIMARY KEY,
    mailbox_id VARCHAR(255) NOT NULL,
    greeting_type VARCHAR(20) NOT NULL CHECK(greeting_type IN ('unavailable', 'busy', 'name', 'temporary')),
    audio_path TEXT NOT NULL, -- Path to greeting audio file
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    expires_at TIMESTAMP NULL, -- For temporary greetings only

    -- Ensure only one greeting per type per mailbox (except temporary can have multiple)
    UNIQUE(mailbox_id, greeting_type)
    -- Foreign keys temporarily disabled for Phase 1
    -- FOREIGN KEY (mailbox_id) REFERENCES voicemail_mailboxes(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_voicemail_greetings_mailbox ON voicemail_greetings(mailbox_id);
CREATE INDEX IF NOT EXISTS idx_voicemail_greetings_type ON voicemail_greetings(mailbox_id, greeting_type);
