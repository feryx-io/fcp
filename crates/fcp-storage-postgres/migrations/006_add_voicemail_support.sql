-- Voicemail support schema
-- Version: 0.3.0
-- This migration adds tables for voicemail mailboxes, messages, and greetings

-- Voicemail mailboxes table
CREATE TABLE IF NOT EXISTS voicemail_mailboxes (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    line_binding_id TEXT NOT NULL,
    extension TEXT NOT NULL,
    pin TEXT NOT NULL, -- hashed PIN for mailbox access
    enabled BOOLEAN NOT NULL DEFAULT true,

    -- Limits
    max_message_duration INTEGER NOT NULL DEFAULT 300, -- 5 minutes
    max_messages INTEGER NOT NULL DEFAULT 100,

    -- Notifications
    email_notification BOOLEAN NOT NULL DEFAULT false,
    email_address TEXT,
    attach_audio BOOLEAN NOT NULL DEFAULT false,
    include_transcription BOOLEAN NOT NULL DEFAULT false,
    transcription_enabled BOOLEAN NOT NULL DEFAULT false,

    -- Audit
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Constraints
    UNIQUE(tenant_id, extension),
    FOREIGN KEY (line_binding_id) REFERENCES line_bindings(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_voicemail_mailboxes_tenant ON voicemail_mailboxes(tenant_id);
CREATE INDEX IF NOT EXISTS idx_voicemail_mailboxes_user ON voicemail_mailboxes(user_id);
CREATE INDEX IF NOT EXISTS idx_voicemail_mailboxes_line_binding ON voicemail_mailboxes(line_binding_id);
CREATE INDEX IF NOT EXISTS idx_voicemail_mailboxes_extension ON voicemail_mailboxes(tenant_id, extension);

-- Voicemail messages table
CREATE TABLE IF NOT EXISTS voicemail_messages (
    id TEXT PRIMARY KEY,
    mailbox_id TEXT NOT NULL,
    caller TEXT NOT NULL, -- E.164 formatted phone number
    caller_name TEXT, -- Caller ID name if available
    status TEXT NOT NULL CHECK(status IN ('new', 'heard', 'saved', 'deleted')),
    priority TEXT NOT NULL CHECK(priority IN ('normal', 'urgent')),
    duration INTEGER NOT NULL, -- Duration in seconds
    recording_path TEXT NOT NULL, -- Path to audio file
    transcription TEXT, -- Optional transcription text

    -- Timestamps
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    accessed_at TIMESTAMPTZ, -- When message was last played/accessed

    FOREIGN KEY (mailbox_id) REFERENCES voicemail_mailboxes(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_voicemail_messages_mailbox ON voicemail_messages(mailbox_id);
CREATE INDEX IF NOT EXISTS idx_voicemail_messages_status ON voicemail_messages(mailbox_id, status);
CREATE INDEX IF NOT EXISTS idx_voicemail_messages_created ON voicemail_messages(mailbox_id, created_at DESC);

-- Voicemail greetings table
CREATE TABLE IF NOT EXISTS voicemail_greetings (
    id BIGSERIAL PRIMARY KEY,
    mailbox_id TEXT NOT NULL,
    greeting_type TEXT NOT NULL CHECK(greeting_type IN ('unavailable', 'busy', 'name', 'temporary')),
    audio_path TEXT NOT NULL, -- Path to greeting audio file
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ, -- For temporary greetings only

    -- Ensure only one greeting per type per mailbox (except temporary can have multiple)
    UNIQUE(mailbox_id, greeting_type),
    FOREIGN KEY (mailbox_id) REFERENCES voicemail_mailboxes(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_voicemail_greetings_mailbox ON voicemail_greetings(mailbox_id);
CREATE INDEX IF NOT EXISTS idx_voicemail_greetings_type ON voicemail_greetings(mailbox_id, greeting_type);
