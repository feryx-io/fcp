//! Voicemail service implementation for FCP.
//!
//! This crate provides voicemail functionality including:
//! - Recording voicemail messages with greeting playback
//! - Message storage and retrieval
//! - Message playback and deletion
//! - Message Waiting Indicator (MWI) notifications
//! - Greeting management

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use anyhow::{Context, Result};
use tracing::{debug, info, instrument, warn};

use fcp_core::storage::VoicemailStore;
use fcp_core::{
    CallId, MessagePriority, MessageStatus, MwiNotifier, VoicemailMailbox, VoicemailMessage,
};
use fcp_media::MediaManager;

/// Configuration for the voicemail service.
#[derive(Debug, Clone)]
pub struct VoicemailConfig {
    /// Directory where voicemail recordings are stored
    pub recordings_dir: PathBuf,

    /// Directory where voicemail greetings are stored.
    /// Currently unused by the service (greetings are stored per-mailbox),
    /// but kept for server-level configuration.
    pub greetings_dir: PathBuf,

    /// Maximum message duration in seconds (default: 300 = 5 minutes)
    pub max_message_duration: u32,

    /// Maximum number of messages per mailbox (default: 100).
    /// Applied as an upper bound even if the individual mailbox allows more.
    pub max_messages_per_mailbox: u32,

    /// Play beep before recording (default: true)
    pub play_beep: bool,

    /// Path to default system greeting audio file
    pub default_greeting_path: Option<PathBuf>,

    /// Path to beep audio file
    pub beep_audio_path: Option<PathBuf>,
}

impl Default for VoicemailConfig {
    fn default() -> Self {
        Self {
            recordings_dir: PathBuf::from("/var/fcp/voicemail/recordings"),
            greetings_dir: PathBuf::from("/var/fcp/voicemail/greetings"),
            max_message_duration: 300, // 5 minutes
            max_messages_per_mailbox: 100,
            play_beep: true,
            default_greeting_path: Some(PathBuf::from("/var/fcp/audio/default-greeting.wav")),
            beep_audio_path: Some(PathBuf::from("/var/fcp/audio/beep.wav")),
        }
    }
}

impl VoicemailConfig {
    /// Validate configuration at construction time.
    ///
    /// Checks that configured paths exist so that misconfiguration is caught
    /// at startup rather than surfacing during the first call.
    pub fn validate(&self) -> Result<()> {
        if !self.recordings_dir.exists() {
            std::fs::create_dir_all(&self.recordings_dir).with_context(|| {
                format!(
                    "Failed to create recordings directory: {:?}",
                    self.recordings_dir
                )
            })?;
        }

        if let Some(ref path) = self.default_greeting_path {
            if !path.exists() {
                warn!(
                    path = ?path,
                    "Default greeting file does not exist — callers will hear an error"
                );
            }
        }

        if self.play_beep {
            if let Some(ref path) = self.beep_audio_path {
                if !path.exists() {
                    warn!(
                        path = ?path,
                        "Beep audio file does not exist — beep will be skipped"
                    );
                }
            }
        }

        if self.max_message_duration == 0 {
            anyhow::bail!("max_message_duration must be > 0");
        }
        if self.max_messages_per_mailbox == 0 {
            anyhow::bail!("max_messages_per_mailbox must be > 0");
        }

        Ok(())
    }
}

/// Voicemail service for managing voicemail messages and mailboxes.
pub struct VoicemailService {
    /// Storage backend for voicemail data
    storage: Arc<dyn VoicemailStore>,

    /// Media manager for playing audio and recording
    media_manager: Arc<MediaManager>,

    /// Optional MWI notifier for sending SIP NOTIFY messages
    mwi_notifier: Option<Arc<dyn MwiNotifier>>,

    /// Configuration
    config: VoicemailConfig,
}

impl VoicemailService {
    /// Creates a new voicemail service.
    pub fn new(
        storage: Arc<dyn VoicemailStore>,
        media_manager: Arc<MediaManager>,
        config: VoicemailConfig,
    ) -> Self {
        Self {
            storage,
            media_manager,
            mwi_notifier: None,
            config,
        }
    }

    /// Creates a new voicemail service with an MWI notifier.
    pub fn with_mwi_notifier(
        storage: Arc<dyn VoicemailStore>,
        media_manager: Arc<MediaManager>,
        mwi_notifier: Arc<dyn MwiNotifier>,
        config: VoicemailConfig,
    ) -> Self {
        Self {
            storage,
            media_manager,
            mwi_notifier: Some(mwi_notifier),
            config,
        }
    }

    /// Records a voicemail message for a mailbox.
    ///
    /// This method:
    /// 1. Retrieves the mailbox configuration
    /// 2. Plays the appropriate greeting (busy or unavailable)
    /// 3. Plays a beep
    /// 4. Records the caller's message
    /// 5. Stores the message to disk and database
    /// 6. Sends MWI notification
    ///
    /// # Arguments
    ///
    /// * `call_id` - The call ID for the current call
    /// * `mailbox_id` - The mailbox ID to leave a message for
    /// * `caller` - The caller's phone number (E.164 format)
    /// * `caller_name` - Optional caller name from caller ID
    /// * `is_busy` - Whether to play busy greeting instead of unavailable
    ///
    /// # Returns
    ///
    /// Returns the created `VoicemailMessage` on success.
    #[instrument(skip(self), fields(
        call_id = %call_id,
        mailbox_id = %mailbox_id,
        caller = %caller,
    ))]
    pub async fn record_message(
        &self,
        call_id: &CallId,
        mailbox_id: &str,
        caller: String,
        caller_name: Option<String>,
        is_busy: bool,
    ) -> Result<VoicemailMessage> {
        info!("Starting voicemail recording");

        // Validate mailbox_id to prevent path traversal
        validate_identifier(mailbox_id, "mailbox_id")?;

        // Get mailbox configuration
        let mailbox = self
            .storage
            .get_mailbox(mailbox_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Mailbox not found: {}", mailbox_id))?;

        if !mailbox.enabled {
            anyhow::bail!("Mailbox is disabled: {}", mailbox_id);
        }

        // Check message limit — enforce the stricter of mailbox vs global config
        let effective_limit =
            std::cmp::min(mailbox.max_messages, self.config.max_messages_per_mailbox);
        let current_messages = self.storage.list_messages(mailbox_id).await?;
        if current_messages.len() >= effective_limit as usize {
            anyhow::bail!("Mailbox is full: {}", mailbox_id);
        }

        // Play greeting
        self.play_greeting(call_id, &mailbox, is_busy).await?;

        // Play beep tone before recording
        if self.config.play_beep {
            debug!("Playing 1000Hz beep tone before recording");
            let playback_id = self
                .media_manager
                .play_tone(
                    call_id,
                    1000.0,  // 1kHz tone
                    300,     // 300ms duration
                    0.5,     // -6dB amplitude
                    fcp_media::AudioTarget::ParticipantA,
                )
                .await
                .context("Failed to start beep tone")?;
            self.media_manager
                .wait_playback_completion(playback_id)
                .await
                .context("Beep tone playback failed")?;
        }

        // Generate recording file path with safe components
        let tenant_dir = mailbox.tenant_id.to_string();
        validate_identifier(&tenant_dir, "tenant_id")?;

        let recording_filename = format!("{}-{}.wav", mailbox_id, uuid::Uuid::new_v4());
        let recording_path = self
            .config
            .recordings_dir
            .join(&tenant_dir)
            .join(mailbox_id)
            .join(&recording_filename);

        // Ensure the constructed path stays within recordings_dir
        validate_path_within_dir(&recording_path, &self.config.recordings_dir)?;

        // Ensure directory exists
        if let Some(parent) = recording_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .context("Failed to create recordings directory")?;
        }

        // Start recording — pass a relative path (tenant/mailbox/file.wav) since
        // the MediaManager prepends its own recording_dir base.
        let relative_recording_path = PathBuf::from(&tenant_dir)
            .join(mailbox_id)
            .join(&recording_filename);
        debug!("Starting recording to: {:?} (relative: {:?})", recording_path, relative_recording_path);
        let max_duration = std::cmp::min(
            mailbox.max_message_duration,
            self.config.max_message_duration,
        );

        let recording_start = std::time::Instant::now();

        self.media_manager
            .start_recording(call_id, relative_recording_path.to_string_lossy().to_string())
            .await
            .context("Failed to start recording")?;

        // Wait for recording to complete: poll media session state and break early
        // on hangup, with a hard timeout at max_duration.
        let poll_interval = tokio::time::Duration::from_secs(1);
        let deadline =
            tokio::time::Instant::now() + tokio::time::Duration::from_secs(max_duration as u64);

        loop {
            tokio::select! {
                _ = tokio::time::sleep_until(deadline) => {
                    // Hard timeout reached
                    debug!("Recording reached max duration ({max_duration}s)");
                    break;
                }
                _ = tokio::time::sleep(poll_interval) => {
                    // Check if the media session is still active
                    match self.media_manager.get_session(call_id).await {
                        Ok(session) => {
                            let state = session.state().await;
                            if state == fcp_media::MediaSessionState::Terminated {
                                info!("Media session terminated — stopping recording early");
                                break;
                            }
                        }
                        Err(_) => {
                            // Session gone — caller hung up
                            info!("Media session not found — stopping recording early");
                            break;
                        }
                    }
                }
            }
        }

        // Stop recording (ignore errors if session already terminated)
        if let Err(e) = self.media_manager.stop_recording(call_id).await {
            debug!("stop_recording returned error (session may have ended): {e}");
        }

        // Calculate actual recording duration from wall clock
        let duration = recording_start.elapsed().as_secs() as u32;

        info!(duration_secs = duration, "Recording completed");

        // Create message record
        let message = VoicemailMessage {
            id: uuid::Uuid::new_v4().to_string(),
            mailbox_id: mailbox_id.to_string(),
            caller: caller.clone(),
            caller_name: caller_name.clone(),
            status: MessageStatus::New,
            priority: MessagePriority::Normal,
            duration,
            recording_path,
            transcription: None,
            created_at: SystemTime::now(),
            accessed_at: None,
        };

        // Store message in database
        self.storage
            .put_message(message.clone())
            .await
            .context("Failed to store message")?;

        info!(message_id = %message.id, "Voicemail message saved");

        // Send MWI notification
        if let Err(e) = self.send_mwi(mailbox_id).await {
            warn!("Failed to send MWI notification: {}", e);
            // Don't fail the whole operation if MWI fails
        }

        Ok(message)
    }

    /// Retrieves messages for a mailbox.
    ///
    /// # Arguments
    ///
    /// * `mailbox_id` - The mailbox ID
    /// * `status` - Optional status filter (New, Heard, Saved)
    ///
    /// # Returns
    ///
    /// Returns a vector of `VoicemailMessage` sorted by creation time (newest first).
    #[instrument(skip(self))]
    pub async fn get_messages(
        &self,
        mailbox_id: &str,
        status: Option<MessageStatus>,
    ) -> Result<Vec<VoicemailMessage>> {
        debug!("Retrieving messages for mailbox: {}", mailbox_id);

        let messages = match status {
            Some(status) => {
                self.storage
                    .list_messages_by_status(mailbox_id, status)
                    .await?
            }
            None => self.storage.list_messages(mailbox_id).await?,
        };

        Ok(messages)
    }

    /// Plays a voicemail message to the caller.
    ///
    /// # Arguments
    ///
    /// * `call_id` - The call ID for the current call
    /// * `message_id` - The message ID to play
    ///
    /// # Returns
    ///
    /// Returns Ok(()) if playback succeeds, or an error if the message is not found
    /// or playback fails.
    #[instrument(skip(self), fields(call_id = %call_id, message_id = %message_id))]
    pub async fn play_message(&self, call_id: &CallId, message_id: &str) -> Result<()> {
        debug!("Playing message");

        // Get message
        let mut message = self
            .storage
            .get_message(message_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Message not found: {}", message_id))?;

        // Validate recording path is within allowed directory before playing
        validate_path_within_dir(&message.recording_path, &self.config.recordings_dir)?;

        // Play the recording
        self.play_audio(call_id, &message.recording_path).await?;

        // Mark as heard if it was new
        if message.status == MessageStatus::New {
            message.mark_heard();
            self.storage
                .put_message(message.clone())
                .await
                .context("Failed to update message status")?;

            // Update MWI after marking as heard
            if let Err(e) = self.send_mwi(&message.mailbox_id).await {
                warn!("Failed to send MWI notification: {}", e);
            }
        }

        info!("Message playback completed");
        Ok(())
    }

    /// Deletes a voicemail message.
    ///
    /// # Arguments
    ///
    /// * `message_id` - The message ID to delete
    ///
    /// # Returns
    ///
    /// Returns Ok(()) if deletion succeeds.
    #[instrument(skip(self))]
    pub async fn delete_message(&self, message_id: &str) -> Result<()> {
        debug!("Deleting message");

        // Get message to check mailbox ID for MWI update
        let message = self
            .storage
            .get_message(message_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Message not found: {}", message_id))?;

        let mailbox_id = message.mailbox_id.clone();

        // Validate recording path is within allowed directory before deletion
        validate_path_within_dir(&message.recording_path, &self.config.recordings_dir)?;

        // Delete from database (CASCADE will handle file cleanup)
        self.storage
            .delete_message(message_id)
            .await
            .context("Failed to delete message from database")?;

        // Delete audio file
        if message.recording_path.exists() {
            tokio::fs::remove_file(&message.recording_path)
                .await
                .context("Failed to delete recording file")?;
        }

        info!("Message deleted successfully");

        // Update MWI
        if let Err(e) = self.send_mwi(&mailbox_id).await {
            warn!("Failed to send MWI notification: {}", e);
        }

        Ok(())
    }

    /// Sends a Message Waiting Indicator (MWI) notification for a mailbox.
    ///
    /// This will trigger a SIP NOTIFY message to the line binding associated
    /// with the mailbox, informing the phone about new messages.
    ///
    /// # Arguments
    ///
    /// * `mailbox_id` - The mailbox ID to send MWI for
    ///
    /// # Returns
    ///
    /// Returns Ok(()) if the MWI notification was sent successfully.
    #[instrument(skip(self))]
    pub async fn send_mwi(&self, mailbox_id: &str) -> Result<()> {
        debug!("Sending MWI notification");

        // Get MWI status
        let mwi_status = self.storage.get_mwi_status(mailbox_id).await?;

        info!(
            new_messages = mwi_status.new_messages,
            old_messages = mwi_status.old_messages,
            urgent_new = mwi_status.urgent_new_messages,
            "MWI status retrieved"
        );

        match &self.mwi_notifier {
            Some(notifier) => {
                notifier.notify_mwi(&mwi_status).await?;
            }
            None => {
                warn!("MWI notifier not configured — skipping SIP NOTIFY");
            }
        }

        Ok(())
    }

    /// Resolves a mailbox by ID or extension.
    ///
    /// Tries lookup by mailbox ID first, then falls back to extension-based
    /// lookup (using the nil tenant).
    pub async fn resolve_mailbox(&self, id_or_extension: &str) -> Result<Option<VoicemailMailbox>> {
        if let Some(mb) = self.storage.get_mailbox(id_or_extension).await? {
            return Ok(Some(mb));
        }
        debug!(id_or_extension = %id_or_extension, "Mailbox not found by ID, trying extension lookup");
        self.storage
            .get_mailbox_by_extension(&fcp_core::TenantId::nil(), id_or_extension)
            .await
    }

    /// Verifies a PIN against a mailbox.
    ///
    /// Tries lookup by mailbox ID first, then falls back to extension-based
    /// lookup (using the nil tenant). This allows callers to pass either a
    /// mailbox UUID or an extension number.
    ///
    /// # Returns
    /// - `Ok(Some(mailbox_id))` if the PIN matches (returns the resolved mailbox ID)
    /// - `Ok(None)` if the PIN is wrong or the mailbox is not found/disabled
    /// - `Err(...)` on storage errors
    pub async fn verify_pin(&self, mailbox_id: &str, pin: &str) -> Result<Option<String>> {
        let mailbox = match self.resolve_mailbox(mailbox_id).await? {
            Some(mb) => mb,
            None => return Ok(None),
        };

        if !mailbox.enabled {
            return Ok(None);
        }

        if mailbox.verify_pin(pin) {
            Ok(Some(mailbox.id))
        } else {
            Ok(None)
        }
    }

    /// Saves a message by updating its status to Saved.
    ///
    /// This persists the updated message via the storage backend.
    pub async fn save_message(&self, message: &VoicemailMessage) -> Result<()> {
        let mut updated = message.clone();
        updated.status = MessageStatus::Saved;
        self.storage.put_message(updated).await
    }

    /// Plays a greeting for a mailbox.
    ///
    /// Selects the appropriate greeting based on the busy status and plays it to the caller.
    /// Falls back to the system default greeting if no custom greeting is available.
    async fn play_greeting(
        &self,
        call_id: &CallId,
        mailbox: &VoicemailMailbox,
        is_busy: bool,
    ) -> Result<()> {
        debug!("Playing greeting (busy: {})", is_busy);

        // Get appropriate greeting
        let greeting = mailbox.get_greeting(is_busy);

        let audio_path = match greeting {
            Some(greeting) => {
                // Custom greeting — validate it's within a safe directory
                validate_path_within_dir(
                    &greeting.audio_path,
                    &self.config.greetings_dir,
                )
                .context("Custom greeting path failed safety check")?;
                &greeting.audio_path
            }
            None => {
                // Fall back to system default greeting
                self.config.default_greeting_path.as_ref().ok_or_else(|| {
                    anyhow::anyhow!("No greeting available and no default configured")
                })?
            }
        };

        self.play_audio(call_id, audio_path).await
    }

    /// Plays an audio file to the caller.
    async fn play_audio(&self, call_id: &CallId, audio_path: &PathBuf) -> Result<()> {
        debug!("Playing audio file: {:?}", audio_path);

        if !audio_path.exists() {
            anyhow::bail!("Audio file not found: {:?}", audio_path);
        }

        let playback_id = self
            .media_manager
            .play_file(
                call_id,
                audio_path.to_string_lossy().to_string(),
                fcp_media::AudioTarget::ParticipantA,
            )
            .await
            .context("Failed to start audio playback")?;

        self.media_manager
            .wait_playback_completion(playback_id)
            .await
            .context("Audio playback failed")?;

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Validation helpers
// ---------------------------------------------------------------------------

/// Validates that an identifier (mailbox_id, tenant_id, etc.) contains only
/// safe characters: `[a-zA-Z0-9_-]`. This prevents path traversal via crafted
/// IDs containing `../`, `/`, null bytes, or other dangerous sequences.
fn validate_identifier(id: &str, field_name: &str) -> Result<()> {
    if id.is_empty() {
        anyhow::bail!("{field_name} must not be empty");
    }
    if id.len() > 128 {
        anyhow::bail!("{field_name} must be 128 characters or fewer");
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        anyhow::bail!(
            "{field_name} contains unsafe characters (only a-z, A-Z, 0-9, -, _ are allowed): {:?}",
            id
        );
    }
    Ok(())
}

/// Ensures `path` resolves to a location within `base_dir`.
///
/// If the path already exists on disk we canonicalize both sides and compare.
/// If the path does not yet exist (e.g. a recording about to be written) we
/// canonicalize the nearest existing ancestor and verify the logical join
/// stays inside.
fn validate_path_within_dir(path: &Path, base_dir: &Path) -> Result<()> {
    // Canonicalize base_dir (must exist)
    let canonical_base = base_dir.canonicalize().with_context(|| {
        format!(
            "Cannot canonicalize base directory {:?} — does it exist?",
            base_dir
        )
    })?;

    if path.exists() {
        // Path exists — full canonicalize
        let canonical_path = path
            .canonicalize()
            .with_context(|| format!("Cannot canonicalize path {:?}", path))?;
        if !canonical_path.starts_with(&canonical_base) {
            anyhow::bail!(
                "Path {:?} is outside allowed directory {:?}",
                canonical_path,
                canonical_base,
            );
        }
    } else {
        // Path does not exist yet. Walk up to find the nearest existing
        // ancestor, canonicalize that, and check the remainder is benign.
        let mut ancestor = path.to_path_buf();
        let mut suffix_parts: Vec<std::ffi::OsString> = Vec::new();

        loop {
            if ancestor.exists() {
                break;
            }
            if let Some(name) = ancestor.file_name() {
                suffix_parts.push(name.to_os_string());
            } else {
                break;
            }
            if !ancestor.pop() {
                break;
            }
        }

        let canonical_ancestor = ancestor.canonicalize().with_context(|| {
            format!("Cannot canonicalize nearest ancestor of {:?}", path)
        })?;

        // Re-assemble the full logical path
        let mut full = canonical_ancestor;
        for part in suffix_parts.iter().rev() {
            // Reject ".." or absolute components in the suffix
            let s = part.to_string_lossy();
            if s == ".." || s.contains('/') || s.contains('\\') || s.contains('\0') {
                anyhow::bail!(
                    "Path component {:?} is unsafe (potential path traversal)",
                    s,
                );
            }
            full.push(part);
        }

        if !full.starts_with(&canonical_base) {
            anyhow::bail!(
                "Path {:?} would resolve outside allowed directory {:?}",
                full,
                canonical_base,
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: Full integration tests require storage backend and media manager mocks
    // These would be added in a separate test module with proper mocking

    #[test]
    fn test_config_defaults() {
        let config = VoicemailConfig::default();
        assert_eq!(config.max_message_duration, 300);
        assert_eq!(config.max_messages_per_mailbox, 100);
        assert!(config.play_beep);
    }

    #[test]
    fn test_validate_identifier_ok() {
        assert!(validate_identifier("user-123", "test").is_ok());
        assert!(validate_identifier("tenant-abc_def", "test").is_ok());
        assert!(validate_identifier("A1b2C3", "test").is_ok());
    }

    #[test]
    fn test_validate_identifier_rejects_path_traversal() {
        assert!(validate_identifier("../etc", "test").is_err());
        assert!(validate_identifier("foo/bar", "test").is_err());
        assert!(validate_identifier("foo\\bar", "test").is_err());
        assert!(validate_identifier("", "test").is_err());
    }

    #[test]
    fn test_validate_identifier_rejects_long() {
        let long = "a".repeat(129);
        assert!(validate_identifier(&long, "test").is_err());
        let ok = "a".repeat(128);
        assert!(validate_identifier(&ok, "test").is_ok());
    }

    #[test]
    fn test_validate_path_within_dir_rejects_escape() {
        let base = std::env::temp_dir().join("vm-test-base");
        std::fs::create_dir_all(&base).unwrap();

        // Path that tries to escape via ".."
        let bad_path = base.join("..").join("etc").join("passwd");
        assert!(validate_path_within_dir(&bad_path, &base).is_err());

        // Path that stays inside
        let good_path = base.join("tenant-1").join("msg.wav");
        assert!(validate_path_within_dir(&good_path, &base).is_ok());

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn test_validate_path_within_dir_existing_file() {
        let base = std::env::temp_dir().join("vm-test-existing");
        let sub = base.join("sub");
        std::fs::create_dir_all(&sub).unwrap();

        let file = sub.join("test.wav");
        std::fs::write(&file, b"test").unwrap();

        // Existing file within base — ok
        assert!(validate_path_within_dir(&file, &base).is_ok());

        // /etc/passwd is outside base
        let etc_passwd = Path::new("/etc/passwd");
        if etc_passwd.exists() {
            assert!(validate_path_within_dir(etc_passwd, &base).is_err());
        }

        std::fs::remove_dir_all(&base).ok();
    }
}
