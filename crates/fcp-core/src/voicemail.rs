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

    /// The site holding the file, at the cloud: the row came up its link
    /// (`docs/HYBRID_CONTROL_PLANE.md` §3.3), and the file is fetched
    /// through it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub site_id: Option<crate::SiteId>,

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
            site_id: None,
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

    /// Whether this greeting is active (considered during playback)
    pub is_active: bool,

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
            is_active: true,
            created_at: SystemTime::now(),
            expires_at: None,
        }
    }

    /// Creates a new greeting with an expiration time.
    pub fn with_expiration(
        greeting_type: GreetingType,
        audio_path: PathBuf,
        expires_at: SystemTime,
    ) -> Self {
        Self {
            greeting_type,
            audio_path,
            is_active: true,
            created_at: SystemTime::now(),
            expires_at: Some(expires_at),
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

    /// The line the mailbox answers for; none once the line is released
    /// (a purged user's mailbox, kept until retention empties it, or a
    /// line deleted on its own).
    pub line_binding_id: Option<LineBindingId>,

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

    /// Message retention period in days (None = keep forever)
    pub retention_days: Option<u32>,

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
            line_binding_id: Some(line_binding_id),
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
            retention_days: None,
            greetings: Vec::new(),
            created_at: SystemTime::now(),
            updated_at: SystemTime::now(),
        }
    }

    /// Gets the appropriate greeting for the given scenario.
    ///
    /// Priority: busy greeting (if caller got busy) → alternate/temporary
    /// (if active and not expired) → standard/unavailable (if active).
    /// Busy greetings are always played when present and the caller is busy;
    /// they do not use the `is_active` flag.
    pub fn get_greeting(&self, is_busy: bool) -> Option<&VoicemailGreeting> {
        // Check for busy greeting first (always played if present, no is_active check)
        if is_busy {
            if let Some(greeting) = self
                .greetings
                .iter()
                .find(|g| g.greeting_type == GreetingType::Busy)
            {
                return Some(greeting);
            }
        }

        // Check for alternate/temporary greeting (must be active and not expired)
        if let Some(greeting) = self
            .greetings
            .iter()
            .find(|g| g.greeting_type == GreetingType::Temporary && g.is_active && !g.is_expired())
        {
            return Some(greeting);
        }

        // Fall back to standard/unavailable greeting (must be active)
        self.greetings
            .iter()
            .find(|g| g.greeting_type == GreetingType::Unavailable && g.is_active)
    }

    /// Sets the active state of a greeting by type.
    ///
    /// Standard (Unavailable) and Alternate (Temporary) are mutually exclusive:
    /// activating one automatically deactivates the other. At least one of them
    /// must always be active — deactivating one activates the other.
    pub fn set_greeting_active(&mut self, greeting_type: GreetingType, active: bool) {
        let has_greeting = self
            .greetings
            .iter()
            .any(|g| g.greeting_type == greeting_type);
        if !has_greeting {
            return;
        }

        // Enforce mutual exclusivity between Unavailable and Temporary
        let counterpart = match greeting_type {
            GreetingType::Unavailable => Some(GreetingType::Temporary),
            GreetingType::Temporary => Some(GreetingType::Unavailable),
            _ => None,
        };

        if active {
            // Activating one deactivates the other
            if let Some(other) = counterpart {
                if let Some(g) = self.greetings.iter_mut().find(|g| g.greeting_type == other) {
                    g.is_active = false;
                }
            }
            if let Some(g) = self
                .greetings
                .iter_mut()
                .find(|g| g.greeting_type == greeting_type)
            {
                g.is_active = true;
            }
        } else {
            // Deactivating one activates the other (one must always be active)
            if let Some(g) = self
                .greetings
                .iter_mut()
                .find(|g| g.greeting_type == greeting_type)
            {
                g.is_active = false;
            }
            if let Some(other) = counterpart {
                if let Some(g) = self.greetings.iter_mut().find(|g| g.greeting_type == other) {
                    g.is_active = true;
                }
            }
        }

        self.updated_at = SystemTime::now();
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

    /// Line binding ID, when the mailbox has a line
    pub line_binding_id: Option<LineBindingId>,

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
    pub fn new(mailbox_id: String, line_binding_id: Option<LineBindingId>) -> Self {
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

/// Trait for sending MWI (Message Waiting Indicator) notifications.
///
/// Implementations typically send a SIP NOTIFY with an
/// `application/simple-message-summary` body per RFC 3842.
#[async_trait::async_trait]
pub trait MwiNotifier: Send + Sync {
    /// Send an MWI notification for the given status.
    ///
    /// The implementation is responsible for resolving the target
    /// endpoint(s) and delivering the notification.
    async fn notify_mwi(&self, mwi_status: &MwiStatus) -> anyhow::Result<()>;
}

/// Voicemail statistics.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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
        let mut mwi = MwiStatus::new("mailbox-123".to_string(), Some(LineBindingId::new()));

        assert!(!mwi.has_messages());

        mwi.new_messages = 3;
        assert!(mwi.has_messages());

        mwi.new_messages = 0;
        mwi.urgent_new_messages = 1;
        assert!(mwi.has_messages());
    }

    // ---- Helpers ----

    fn test_mailbox() -> VoicemailMailbox {
        VoicemailMailbox::new(
            TenantId::nil(),
            UserId::new(),
            LineBindingId::new(),
            "1001".to_string(),
            "1234".to_string(),
        )
    }

    fn expired_greeting(greeting_type: GreetingType) -> VoicemailGreeting {
        VoicemailGreeting {
            greeting_type,
            audio_path: PathBuf::from("/tmp/expired.wav"),
            is_active: true,
            created_at: SystemTime::now(),
            expires_at: Some(SystemTime::UNIX_EPOCH), // already expired
        }
    }

    // ---- set_greeting_active mutual exclusivity ----

    #[test]
    fn test_set_greeting_active_activating_temporary_deactivates_unavailable() {
        let mut mailbox = test_mailbox();
        mailbox.add_greeting(VoicemailGreeting::new(
            GreetingType::Unavailable,
            PathBuf::from("/tmp/unavail.wav"),
        ));
        mailbox.add_greeting(VoicemailGreeting::new(
            GreetingType::Temporary,
            PathBuf::from("/tmp/temp.wav"),
        ));
        // Both start active; explicitly activate Temporary
        mailbox.set_greeting_active(GreetingType::Temporary, true);

        let temp = mailbox
            .greetings
            .iter()
            .find(|g| g.greeting_type == GreetingType::Temporary)
            .unwrap();
        let unavail = mailbox
            .greetings
            .iter()
            .find(|g| g.greeting_type == GreetingType::Unavailable)
            .unwrap();
        assert!(temp.is_active);
        assert!(!unavail.is_active);
    }

    #[test]
    fn test_set_greeting_active_activating_unavailable_deactivates_temporary() {
        let mut mailbox = test_mailbox();
        mailbox.add_greeting(VoicemailGreeting::new(
            GreetingType::Unavailable,
            PathBuf::from("/tmp/unavail.wav"),
        ));
        mailbox.add_greeting(VoicemailGreeting::new(
            GreetingType::Temporary,
            PathBuf::from("/tmp/temp.wav"),
        ));
        mailbox.set_greeting_active(GreetingType::Unavailable, true);

        let temp = mailbox
            .greetings
            .iter()
            .find(|g| g.greeting_type == GreetingType::Temporary)
            .unwrap();
        let unavail = mailbox
            .greetings
            .iter()
            .find(|g| g.greeting_type == GreetingType::Unavailable)
            .unwrap();
        assert!(unavail.is_active);
        assert!(!temp.is_active);
    }

    #[test]
    fn test_set_greeting_active_deactivating_unavailable_activates_temporary() {
        let mut mailbox = test_mailbox();
        mailbox.add_greeting(VoicemailGreeting::new(
            GreetingType::Unavailable,
            PathBuf::from("/tmp/unavail.wav"),
        ));
        let mut temp =
            VoicemailGreeting::new(GreetingType::Temporary, PathBuf::from("/tmp/temp.wav"));
        temp.is_active = false;
        mailbox.greetings.push(temp);

        mailbox.set_greeting_active(GreetingType::Unavailable, false);

        let temp = mailbox
            .greetings
            .iter()
            .find(|g| g.greeting_type == GreetingType::Temporary)
            .unwrap();
        let unavail = mailbox
            .greetings
            .iter()
            .find(|g| g.greeting_type == GreetingType::Unavailable)
            .unwrap();
        assert!(!unavail.is_active);
        assert!(temp.is_active);
    }

    #[test]
    fn test_set_greeting_active_deactivating_temporary_activates_unavailable() {
        let mut mailbox = test_mailbox();
        let mut unavail =
            VoicemailGreeting::new(GreetingType::Unavailable, PathBuf::from("/tmp/unavail.wav"));
        unavail.is_active = false;
        mailbox.greetings.push(unavail);
        mailbox.add_greeting(VoicemailGreeting::new(
            GreetingType::Temporary,
            PathBuf::from("/tmp/temp.wav"),
        ));

        mailbox.set_greeting_active(GreetingType::Temporary, false);

        let temp = mailbox
            .greetings
            .iter()
            .find(|g| g.greeting_type == GreetingType::Temporary)
            .unwrap();
        let unavail = mailbox
            .greetings
            .iter()
            .find(|g| g.greeting_type == GreetingType::Unavailable)
            .unwrap();
        assert!(!temp.is_active);
        assert!(unavail.is_active);
    }

    #[test]
    fn test_set_greeting_active_busy_no_counterpart() {
        let mut mailbox = test_mailbox();
        mailbox.add_greeting(VoicemailGreeting::new(
            GreetingType::Unavailable,
            PathBuf::from("/tmp/unavail.wav"),
        ));
        mailbox.add_greeting(VoicemailGreeting::new(
            GreetingType::Busy,
            PathBuf::from("/tmp/busy.wav"),
        ));

        // Toggling Busy should not affect Unavailable
        mailbox.set_greeting_active(GreetingType::Busy, false);
        let unavail = mailbox
            .greetings
            .iter()
            .find(|g| g.greeting_type == GreetingType::Unavailable)
            .unwrap();
        assert!(unavail.is_active);

        mailbox.set_greeting_active(GreetingType::Busy, true);
        let unavail = mailbox
            .greetings
            .iter()
            .find(|g| g.greeting_type == GreetingType::Unavailable)
            .unwrap();
        assert!(unavail.is_active);
    }

    #[test]
    fn test_set_greeting_active_nonexistent_greeting_noop() {
        let mut mailbox = test_mailbox();
        mailbox.add_greeting(VoicemailGreeting::new(
            GreetingType::Unavailable,
            PathBuf::from("/tmp/unavail.wav"),
        ));
        // No Temporary greeting exists — should be a no-op, no panic
        mailbox.set_greeting_active(GreetingType::Temporary, true);

        let unavail = mailbox
            .greetings
            .iter()
            .find(|g| g.greeting_type == GreetingType::Unavailable)
            .unwrap();
        assert!(unavail.is_active);
    }

    // ---- get_greeting selection ----

    #[test]
    fn test_get_greeting_no_greetings_returns_none() {
        let mailbox = test_mailbox();
        assert!(mailbox.get_greeting(false).is_none());
        assert!(mailbox.get_greeting(true).is_none());
    }

    #[test]
    fn test_get_greeting_busy_plays_when_present() {
        let mut mailbox = test_mailbox();
        mailbox.add_greeting(VoicemailGreeting::new(
            GreetingType::Unavailable,
            PathBuf::from("/tmp/unavail.wav"),
        ));
        mailbox.add_greeting(VoicemailGreeting::new(
            GreetingType::Busy,
            PathBuf::from("/tmp/busy.wav"),
        ));

        let g = mailbox.get_greeting(true).unwrap();
        assert_eq!(g.greeting_type, GreetingType::Busy);
    }

    #[test]
    fn test_get_greeting_busy_fallback_to_active_greeting() {
        let mut mailbox = test_mailbox();
        mailbox.add_greeting(VoicemailGreeting::new(
            GreetingType::Unavailable,
            PathBuf::from("/tmp/unavail.wav"),
        ));
        // No busy greeting — should fall through to Unavailable
        let g = mailbox.get_greeting(true).unwrap();
        assert_eq!(g.greeting_type, GreetingType::Unavailable);
    }

    #[test]
    fn test_get_greeting_expired_temporary_falls_through() {
        let mut mailbox = test_mailbox();
        mailbox.add_greeting(VoicemailGreeting::new(
            GreetingType::Unavailable,
            PathBuf::from("/tmp/unavail.wav"),
        ));
        mailbox
            .greetings
            .push(expired_greeting(GreetingType::Temporary));

        let g = mailbox.get_greeting(false).unwrap();
        assert_eq!(g.greeting_type, GreetingType::Unavailable);
    }

    #[test]
    fn test_get_greeting_inactive_unavailable_returns_none() {
        let mut mailbox = test_mailbox();
        let mut unavail =
            VoicemailGreeting::new(GreetingType::Unavailable, PathBuf::from("/tmp/unavail.wav"));
        unavail.is_active = false;
        mailbox.greetings.push(unavail);

        // No active Temporary, inactive Unavailable → None
        assert!(mailbox.get_greeting(false).is_none());
    }

    #[test]
    fn test_get_greeting_temporary_overrides_unavailable() {
        let mut mailbox = test_mailbox();
        mailbox.add_greeting(VoicemailGreeting::new(
            GreetingType::Unavailable,
            PathBuf::from("/tmp/unavail.wav"),
        ));
        mailbox.add_greeting(VoicemailGreeting::new(
            GreetingType::Temporary,
            PathBuf::from("/tmp/temp.wav"),
        ));

        let g = mailbox.get_greeting(false).unwrap();
        assert_eq!(g.greeting_type, GreetingType::Temporary);
    }

    #[test]
    fn test_get_greeting_inactive_temporary_returns_unavailable() {
        let mut mailbox = test_mailbox();
        mailbox.add_greeting(VoicemailGreeting::new(
            GreetingType::Unavailable,
            PathBuf::from("/tmp/unavail.wav"),
        ));
        let mut temp =
            VoicemailGreeting::new(GreetingType::Temporary, PathBuf::from("/tmp/temp.wav"));
        temp.is_active = false;
        mailbox.greetings.push(temp);

        let g = mailbox.get_greeting(false).unwrap();
        assert_eq!(g.greeting_type, GreetingType::Unavailable);
    }

    // ---- Other mailbox logic ----

    #[test]
    fn test_add_greeting_replaces_same_type() {
        let mut mailbox = test_mailbox();
        mailbox.add_greeting(VoicemailGreeting::new(
            GreetingType::Unavailable,
            PathBuf::from("/tmp/old.wav"),
        ));
        mailbox.add_greeting(VoicemailGreeting::new(
            GreetingType::Unavailable,
            PathBuf::from("/tmp/new.wav"),
        ));

        assert_eq!(mailbox.greetings.len(), 1);
        assert_eq!(
            mailbox.greetings[0].audio_path,
            PathBuf::from("/tmp/new.wav")
        );
    }

    #[test]
    fn test_verify_pin_correct() {
        let mailbox = test_mailbox();
        assert!(mailbox.verify_pin("1234"));
    }

    #[test]
    fn test_verify_pin_incorrect() {
        let mailbox = test_mailbox();
        assert!(!mailbox.verify_pin("0000"));
        assert!(!mailbox.verify_pin(""));
    }

    #[test]
    fn test_mwi_old_messages_only_not_flagged() {
        let mut mwi = MwiStatus::new("mailbox-123".to_string(), Some(LineBindingId::new()));
        mwi.old_messages = 5;
        mwi.new_messages = 0;
        mwi.urgent_new_messages = 0;
        // has_messages only considers new/urgent_new, not old
        assert!(!mwi.has_messages());
    }
}
