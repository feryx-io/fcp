//! Voicemail service implementation for FCP.
//!
//! This crate provides voicemail functionality including:
//! - Recording voicemail messages with greeting playback
//! - Message storage and retrieval
//! - Message playback and deletion
//! - Message Waiting Indicator (MWI) notifications
//! - Greeting management

use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use anyhow::{Context, Result};
use async_trait::async_trait;
use tracing::{debug, error, info, instrument, warn};

use fcp_core::storage::VoicemailStore;
use fcp_core::{
    CallId, GreetingType, LineBindingId, MessagePriority, MessageStatus, MwiStatus, TenantId,
    VoicemailGreeting, VoicemailMailbox, VoicemailMessage,
};
use fcp_media::MediaManager;

/// Configuration for the voicemail service.
#[derive(Debug, Clone)]
pub struct VoicemailConfig {
    /// Directory where voicemail recordings are stored
    pub recordings_dir: PathBuf,

    /// Directory where voicemail greetings are stored
    pub greetings_dir: PathBuf,

    /// Maximum message duration in seconds (default: 300 = 5 minutes)
    pub max_message_duration: u32,

    /// Maximum number of messages per mailbox (default: 100)
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

/// Voicemail service for managing voicemail messages and mailboxes.
pub struct VoicemailService {
    /// Storage backend for voicemail data
    storage: Arc<dyn VoicemailStore>,

    /// Media manager for playing audio and recording
    media_manager: Arc<MediaManager>,

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

        // Get mailbox configuration
        let mailbox = self
            .storage
            .get_mailbox(mailbox_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Mailbox not found: {}", mailbox_id))?;

        if !mailbox.enabled {
            anyhow::bail!("Mailbox is disabled: {}", mailbox_id);
        }

        // Check message limit
        let current_messages = self.storage.list_messages(mailbox_id).await?;
        if current_messages.len() >= mailbox.max_messages as usize {
            anyhow::bail!("Mailbox is full: {}", mailbox_id);
        }

        // Play greeting
        self.play_greeting(call_id, &mailbox, is_busy).await?;

        // Play beep if configured
        if self.config.play_beep {
            if let Some(ref beep_path) = self.config.beep_audio_path {
                debug!("Playing beep before recording");
                self.play_audio(call_id, beep_path).await?;
            }
        }

        // Generate recording file path
        let recording_filename = format!("{}-{}.wav", mailbox_id, uuid::Uuid::new_v4());
        let recording_path = self
            .config
            .recordings_dir
            .join(&mailbox.tenant_id.to_string())
            .join(mailbox_id)
            .join(&recording_filename);

        // Ensure directory exists
        if let Some(parent) = recording_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .context("Failed to create recordings directory")?;
        }

        // Start recording
        debug!("Starting recording to: {:?}", recording_path);
        let max_duration = std::cmp::min(
            mailbox.max_message_duration,
            self.config.max_message_duration,
        );

        let recording_start = std::time::Instant::now();

        self.media_manager
            .start_recording(call_id, recording_path.to_string_lossy().to_string())
            .await
            .context("Failed to start recording")?;

        // Wait for recording to complete (max duration or caller hangs up)
        // TODO: Implement proper timeout mechanism and early termination on hangup
        tokio::time::sleep(tokio::time::Duration::from_secs(max_duration as u64)).await;

        // Stop recording
        self.media_manager
            .stop_recording(call_id)
            .await
            .context("Failed to stop recording")?;

        // Calculate actual recording duration
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

        // TODO: Send SIP NOTIFY via SIP handler
        // This will be implemented when we add MWI support to fcp-sip
        warn!("MWI notification not yet implemented - requires SIP NOTIFY support");

        Ok(())
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
            Some(greeting) => &greeting.audio_path,
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
    async fn play_audio(&self, _call_id: &CallId, audio_path: &PathBuf) -> Result<()> {
        debug!("Playing audio file: {:?}", audio_path);

        if !audio_path.exists() {
            anyhow::bail!("Audio file not found: {:?}", audio_path);
        }

        // TODO: Implement audio playback via MediaManager
        // For now, this is a placeholder
        warn!("Audio playback not yet fully implemented in MediaManager");

        // self.media_manager
        //     .play_audio(call_id, audio_path)
        //     .await
        //     .context("Failed to play audio")?;

        Ok(())
    }
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
}
