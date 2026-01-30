//! Voicemail system for storing and retrieving voice messages.
//!
//! This module provides voicemail functionality including:
//! - Mailbox management
//! - Message storage and retrieval
//! - Greeting management (personal, busy, unavailable)
//! - Message waiting indicator (MWI) notifications
//! - Transcription support

use crate::{LineBindingId, TenantId, UserId};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::SystemTime;

/// Voicemail message status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageStatus {
    /// New, unheard message
    New,
    /// Message has been listened to
    Heard,
    /// Message saved for later
    Saved,
    /// Message marked for deletion
    Deleted,
}

impl std::fmt::Display for MessageStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MessageStatus::New => write!(f, "new"),
            MessageStatus::Heard => write!(f, "heard"),
            MessageStatus::Saved => write!(f, "saved"),
            MessageStatus::Deleted => write!(f, "deleted"),
        }
    }
}

/// Voicemail message priority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessagePriority {
    /// Normal priority
    Normal,
    /// Urgent/important message
    Urgent,
}

/// A voicemail message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoicemailMessage {
    /// Unique message ID
    pub id: String,

    /// Mailbox ID this message belongs to
    pub mailbox_id: String,

    /// Caller phone number (E.164 format)
    pub caller: String,

    /// Caller name (if available from caller ID)
    pub caller_name: Option<String>,

    /// Message status
    pub status: MessageStatus,

    /// Message priority
    pub priority: MessagePriority,

    /// Duration in seconds
    pub duration: u32,

    /// File path to the audio recording
    pub recording_path: PathBuf,

    /// Optional transcription of the message
    pub transcription: Option<String>,

    /// When the message was left
    pub created_at: SystemTime,

    /// When the message was last accessed
    pub accessed_at: Option<SystemTime>,
}

impl VoicemailMessage {
    /// Creates a new voicemail message.
    pub fn new(mailbox_id: String, caller: String, duration: u32, recording_path: PathBuf) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            mailbox_id,
            caller,
            caller_name: None,
            status: MessageStatus::New,
            priority: MessagePriority::Normal,
            duration,
            recording_path,
            transcription: None,
            created_at: SystemTime::now(),
            accessed_at: None,
        }
    }

    /// Marks the message as heard.
    pub fn mark_heard(&mut self) {
        self.status = MessageStatus::Heard;
        self.accessed_at = Some(SystemTime::now());
    }

    /// Marks the message as saved.
    pub fn mark_saved(&mut self) {
        self.status = MessageStatus::Saved;
        self.accessed_at = Some(SystemTime::now());
    }

    /// Marks the message for deletion.
    pub fn mark_deleted(&mut self) {
        self.status = MessageStatus::Deleted;
    }
}

/// Greeting type for voicemail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GreetingType {
    /// Standard greeting (unavailable)
    Unavailable,
    /// Busy greeting
    Busy,
    /// Personal name greeting
    Name,
    /// Temporary greeting (overrides others when set)
    Temporary,
}

impl std::fmt::Display for GreetingType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GreetingType::Unavailable => write!(f, "unavailable"),
            GreetingType::Busy => write!(f, "busy"),
            GreetingType::Name => write!(f, "name"),
            GreetingType::Temporary => write!(f, "temporary"),
        }
    }
}

/// A voicemail greeting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoicemailGreeting {
    /// Greeting type
    pub greeting_type: GreetingType,

    /// File path to the audio greeting
    pub audio_path: PathBuf,

    /// When this greeting was recorded
    pub created_at: SystemTime,

    /// Expiration for temporary greetings
    pub expires_at: Option<SystemTime>,
}

impl VoicemailGreeting {
    /// Creates a new greeting.
    pub fn new(greeting_type: GreetingType, audio_path: PathBuf) -> Self {
        Self {
            greeting_type,
            audio_path,
            created_at: SystemTime::now(),
            expires_at: None,
        }
    }

    /// Checks if this greeting has expired (for temporary greetings).
    pub fn is_expired(&self) -> bool {
        if let Some(expires_at) = self.expires_at {
            SystemTime::now() > expires_at
        } else {
            false
        }
    }
}

/// Voicemail mailbox configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoicemailMailbox {
    /// Unique mailbox ID
    pub id: String,

    /// Tenant this mailbox belongs to
    pub tenant_id: TenantId,

    /// User ID that owns this mailbox
    pub user_id: UserId,

    /// Line binding associated with this mailbox
    pub line_binding_id: LineBindingId,

    /// Extension/mailbox number
    pub extension: String,

    /// PIN for accessing the mailbox
    pub pin: String,

    /// Whether the mailbox is enabled
    pub enabled: bool,

    /// Maximum message duration in seconds
    pub max_message_duration: u32,

    /// Maximum number of messages
    pub max_messages: u32,

    /// Whether to send email notifications
    pub email_notification: bool,

    /// Email address for notifications
    pub email_address: Option<String>,

    /// Whether to attach the audio file to email
    pub attach_audio: bool,

    /// Whether to include transcription in email
    pub include_transcription: bool,

    /// Whether to enable message transcription
    pub transcription_enabled: bool,

    /// Greetings for this mailbox
    pub greetings: Vec<VoicemailGreeting>,

    /// Created timestamp
    pub created_at: SystemTime,

    /// Updated timestamp
    pub updated_at: SystemTime,
}

impl VoicemailMailbox {
    /// Creates a new voicemail mailbox.
    pub fn new(
        tenant_id: TenantId,
        user_id: UserId,
        line_binding_id: LineBindingId,
        extension: String,
        pin: String,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            tenant_id,
            user_id,
            line_binding_id,
            extension,
            pin,
            enabled: true,
            max_message_duration: 300, // 5 minutes
            max_messages: 100,
            email_notification: false,
            email_address: None,
            attach_audio: false,
            include_transcription: false,
            transcription_enabled: false,
            greetings: Vec::new(),
            created_at: SystemTime::now(),
            updated_at: SystemTime::now(),
        }
    }

    /// Gets the appropriate greeting for the given scenario.
    pub fn get_greeting(&self, is_busy: bool) -> Option<&VoicemailGreeting> {
        // Check for temporary greeting first
        if let Some(greeting) = self
            .greetings
            .iter()
            .find(|g| g.greeting_type == GreetingType::Temporary && !g.is_expired())
        {
            return Some(greeting);
        }

        // Check for busy greeting if busy
        if is_busy {
            if let Some(greeting) = self
                .greetings
                .iter()
                .find(|g| g.greeting_type == GreetingType::Busy)
            {
                return Some(greeting);
            }
        }

        // Fall back to unavailable greeting
        self.greetings
            .iter()
            .find(|g| g.greeting_type == GreetingType::Unavailable)
    }

    /// Adds a greeting to this mailbox.
    pub fn add_greeting(&mut self, greeting: VoicemailGreeting) {
        // Remove existing greeting of the same type
        self.greetings
            .retain(|g| g.greeting_type != greeting.greeting_type);
        self.greetings.push(greeting);
        self.updated_at = SystemTime::now();
    }

    /// Verifies the PIN.
    pub fn verify_pin(&self, pin: &str) -> bool {
        self.pin == pin
    }
}

/// Message waiting indicator (MWI) status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MwiStatus {
    /// Mailbox ID
    pub mailbox_id: String,

    /// Line binding ID
    pub line_binding_id: LineBindingId,

    /// Number of new messages
    pub new_messages: u32,

    /// Number of old/saved messages
    pub old_messages: u32,

    /// Number of urgent new messages
    pub urgent_new_messages: u32,

    /// Number of urgent old messages
    pub urgent_old_messages: u32,
}

impl MwiStatus {
    /// Creates a new MWI status.
    pub fn new(mailbox_id: String, line_binding_id: LineBindingId) -> Self {
        Self {
            mailbox_id,
            line_binding_id,
            new_messages: 0,
            old_messages: 0,
            urgent_new_messages: 0,
            urgent_old_messages: 0,
        }
    }

    /// Checks if there are any new messages.
    pub fn has_messages(&self) -> bool {
        self.new_messages > 0 || self.urgent_new_messages > 0
    }
}

/// Voicemail statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoicemailStats {
    /// Total messages
    pub total_messages: u32,

    /// New messages
    pub new_messages: u32,

    /// Heard messages
    pub heard_messages: u32,

    /// Saved messages
    pub saved_messages: u32,

    /// Total storage used in bytes
    pub storage_used: u64,

    /// Average message duration
    pub avg_duration: u32,
}

impl Default for VoicemailStats {
    fn default() -> Self {
        Self {
            total_messages: 0,
            new_messages: 0,
            heard_messages: 0,
            saved_messages: 0,
            storage_used: 0,
            avg_duration: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_voicemail_message_creation() {
        let msg = VoicemailMessage::new(
            "mailbox-123".to_string(),
            "+15551234567".to_string(),
            60,
            PathBuf::from("/var/spool/voicemail/msg001.wav"),
        );

        assert_eq!(msg.status, MessageStatus::New);
        assert_eq!(msg.priority, MessagePriority::Normal);
        assert_eq!(msg.duration, 60);
    }

    #[test]
    fn test_message_status_transitions() {
        let mut msg = VoicemailMessage::new(
            "mailbox-123".to_string(),
            "+15551234567".to_string(),
            60,
            PathBuf::from("/var/spool/voicemail/msg001.wav"),
        );

        assert_eq!(msg.status, MessageStatus::New);

        msg.mark_heard();
        assert_eq!(msg.status, MessageStatus::Heard);
        assert!(msg.accessed_at.is_some());

        msg.mark_saved();
        assert_eq!(msg.status, MessageStatus::Saved);

        msg.mark_deleted();
        assert_eq!(msg.status, MessageStatus::Deleted);
    }

    #[test]
    fn test_greeting_selection() {
        let tenant_id = TenantId::nil();
        let user_id = UserId::new();
        let line_binding_id = LineBindingId::new();

        let mut mailbox = VoicemailMailbox::new(
            tenant_id,
            user_id,
            line_binding_id,
            "1001".to_string(),
            "1234".to_string(),
        );

        // Add unavailable greeting
        mailbox.add_greeting(VoicemailGreeting::new(
            GreetingType::Unavailable,
            PathBuf::from("/var/spool/voicemail/greet-unavail.wav"),
        ));

        // Add busy greeting
        mailbox.add_greeting(VoicemailGreeting::new(
            GreetingType::Busy,
            PathBuf::from("/var/spool/voicemail/greet-busy.wav"),
        ));

        // Should get busy greeting when busy
        let greeting = mailbox.get_greeting(true);
        assert!(greeting.is_some());
        assert_eq!(greeting.unwrap().greeting_type, GreetingType::Busy);

        // Should get unavailable greeting when not busy
        let greeting = mailbox.get_greeting(false);
        assert!(greeting.is_some());
        assert_eq!(greeting.unwrap().greeting_type, GreetingType::Unavailable);
    }

    #[test]
    fn test_mwi_status() {
        let mut mwi = MwiStatus::new("mailbox-123".to_string(), LineBindingId::new());

        assert!(!mwi.has_messages());

        mwi.new_messages = 3;
        assert!(mwi.has_messages());

        mwi.new_messages = 0;
        mwi.urgent_new_messages = 1;
        assert!(mwi.has_messages());
    }
}
