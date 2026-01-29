//! Voicemail management API endpoints.

use crate::{error::ApiError, state::ApiState};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use fcp_core::{
    storage::VoicemailStore, LineBindingId, MessageStatus, TenantId, UserId, VoicemailMailbox,
    VoicemailMessage,
};
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use std::time::SystemTime;
use tracing::info;

/// Helper macro to get storage or return error.
macro_rules! require_storage {
    ($state:expr) => {
        $state.storage.as_ref().ok_or_else(|| {
            ApiError::NotFound("Storage is required but not configured".to_string())
        })?
    };
}

/// Voicemail mailbox response.
#[derive(Debug, Serialize)]
pub struct MailboxResponse {
    pub id: String,
    pub tenant_id: String,
    pub user_id: String,
    pub line_binding_id: String,
    pub extension: String,
    pub enabled: bool,
    pub max_message_duration: u32,
    pub max_messages: u32,
    pub email_notification: bool,
    pub email_address: Option<String>,
    pub attach_audio: bool,
    pub include_transcription: bool,
    pub transcription_enabled: bool,
    pub created_at: SystemTime,
    pub updated_at: SystemTime,
}

impl From<VoicemailMailbox> for MailboxResponse {
    fn from(mailbox: VoicemailMailbox) -> Self {
        Self {
            id: mailbox.id,
            tenant_id: mailbox.tenant_id.to_string(),
            user_id: mailbox.user_id.to_string(),
            line_binding_id: mailbox.line_binding_id.to_string(),
            extension: mailbox.extension,
            enabled: mailbox.enabled,
            max_message_duration: mailbox.max_message_duration,
            max_messages: mailbox.max_messages,
            email_notification: mailbox.email_notification,
            email_address: mailbox.email_address,
            attach_audio: mailbox.attach_audio,
            include_transcription: mailbox.include_transcription,
            transcription_enabled: mailbox.transcription_enabled,
            created_at: mailbox.created_at,
            updated_at: mailbox.updated_at,
        }
    }
}

/// Voicemail message response.
#[derive(Debug, Serialize)]
pub struct MessageResponse {
    pub id: String,
    pub mailbox_id: String,
    pub caller: String,
    pub caller_name: Option<String>,
    pub status: String,
    pub priority: String,
    pub duration: u32,
    pub recording_path: String,
    pub transcription: Option<String>,
    pub created_at: SystemTime,
    pub accessed_at: Option<SystemTime>,
}

impl From<VoicemailMessage> for MessageResponse {
    fn from(message: VoicemailMessage) -> Self {
        Self {
            id: message.id,
            mailbox_id: message.mailbox_id,
            caller: message.caller,
            caller_name: message.caller_name,
            status: message.status.to_string(),
            priority: match message.priority {
                fcp_core::MessagePriority::Normal => "normal".to_string(),
                fcp_core::MessagePriority::Urgent => "urgent".to_string(),
            },
            duration: message.duration,
            recording_path: message.recording_path.to_string_lossy().to_string(),
            transcription: message.transcription,
            created_at: message.created_at,
            accessed_at: message.accessed_at,
        }
    }
}

/// Create mailbox request.
#[derive(Debug, Deserialize)]
pub struct CreateMailboxRequest {
    pub tenant_id: String,
    pub user_id: String,
    pub line_binding_id: String,
    pub extension: String,
    pub pin: String,
    #[serde(default = "default_true")]
    #[allow(dead_code)] // TODO: Implement mailbox enable/disable
    pub enabled: bool,
    #[serde(default = "default_max_duration")]
    #[allow(dead_code)] // TODO: Implement message duration limits
    pub max_message_duration: u32,
    #[serde(default = "default_max_messages")]
    #[allow(dead_code)] // TODO: Implement message count limits
    pub max_messages: u32,
}

/// Update mailbox request.
#[derive(Debug, Deserialize)]
pub struct UpdateMailboxRequest {
    pub pin: Option<String>,
    pub enabled: Option<bool>,
    pub max_message_duration: Option<u32>,
    pub max_messages: Option<u32>,
    pub email_notification: Option<bool>,
    pub email_address: Option<String>,
    pub attach_audio: Option<bool>,
    pub include_transcription: Option<bool>,
    pub transcription_enabled: Option<bool>,
}

/// Update message request.
#[derive(Debug, Deserialize)]
pub struct UpdateMessageRequest {
    pub status: Option<String>,
}

fn default_true() -> bool {
    true
}

fn default_max_duration() -> u32 {
    300 // 5 minutes
}

fn default_max_messages() -> u32 {
    100
}

/// List all mailboxes for a tenant.
pub async fn list_mailboxes(
    State(state): State<ApiState>,
    tenant: crate::auth::TenantContext,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);

    let tenant_id = tenant.0;

    let _mailboxes = storage
        .get_mailbox_by_extension(&tenant_id, "")
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    // This is a workaround - ideally we'd have a list_mailboxes method that works properly
    // For now, return empty array since get_mailbox_by_extension won't work without extension
    let mailboxes: Vec<MailboxResponse> = vec![];

    Ok(Json(mailboxes))
}

/// Get a specific mailbox by ID.
pub async fn get_mailbox(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);

    let mailbox = storage
        .get_mailbox(&id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::NotFound(format!("Mailbox {} not found", id)))?;

    Ok(Json(MailboxResponse::from(mailbox)))
}

/// Create a new voicemail mailbox.
pub async fn create_mailbox(
    State(state): State<ApiState>,
    Json(req): Json<CreateMailboxRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);

    let tenant_id = TenantId::from_str(&req.tenant_id)
        .map_err(|e| ApiError::BadRequest(format!("Invalid tenant_id: {}", e)))?;
    let user_id = UserId::from_str(&req.user_id)
        .map_err(|e| ApiError::BadRequest(format!("Invalid user_id: {}", e)))?;
    let line_binding_id = LineBindingId::from_str(&req.line_binding_id)
        .map_err(|e| ApiError::BadRequest(format!("Invalid line_binding_id: {}", e)))?;

    let mailbox = VoicemailMailbox::new(
        tenant_id,
        user_id,
        line_binding_id,
        req.extension.clone(),
        req.pin,
    );

    storage
        .put_mailbox(mailbox.clone())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    info!(mailbox_id = %mailbox.id, extension = %req.extension, "Created voicemail mailbox");

    Ok((StatusCode::CREATED, Json(MailboxResponse::from(mailbox))))
}

/// Update an existing mailbox.
pub async fn update_mailbox(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateMailboxRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);

    let mut mailbox = storage
        .get_mailbox(&id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::NotFound(format!("Mailbox {} not found", id)))?;

    // Update fields if provided
    if let Some(pin) = req.pin {
        mailbox.pin = pin;
    }
    if let Some(enabled) = req.enabled {
        mailbox.enabled = enabled;
    }
    if let Some(max_duration) = req.max_message_duration {
        mailbox.max_message_duration = max_duration;
    }
    if let Some(max_messages) = req.max_messages {
        mailbox.max_messages = max_messages;
    }
    if let Some(email_notification) = req.email_notification {
        mailbox.email_notification = email_notification;
    }
    if let Some(email_address) = req.email_address {
        mailbox.email_address = Some(email_address);
    }
    if let Some(attach_audio) = req.attach_audio {
        mailbox.attach_audio = attach_audio;
    }
    if let Some(include_transcription) = req.include_transcription {
        mailbox.include_transcription = include_transcription;
    }
    if let Some(transcription_enabled) = req.transcription_enabled {
        mailbox.transcription_enabled = transcription_enabled;
    }

    mailbox.updated_at = SystemTime::now();

    storage
        .put_mailbox(mailbox.clone())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    info!(mailbox_id = %id, "Updated voicemail mailbox");

    Ok(Json(MailboxResponse::from(mailbox)))
}

/// Delete a mailbox.
pub async fn delete_mailbox(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);

    storage
        .delete_mailbox(&id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    info!(mailbox_id = %id, "Deleted voicemail mailbox");

    Ok(StatusCode::NO_CONTENT)
}

/// List messages for a mailbox.
pub async fn list_messages(
    State(state): State<ApiState>,
    Path(mailbox_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);

    let messages = storage
        .list_messages(&mailbox_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let responses: Vec<MessageResponse> = messages.into_iter().map(MessageResponse::from).collect();

    Ok(Json(responses))
}

/// Get a specific message.
pub async fn get_message(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);

    let message = storage
        .get_message(&id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::NotFound(format!("Message {} not found", id)))?;

    Ok(Json(MessageResponse::from(message)))
}

/// Update a message (typically to mark as heard/saved).
pub async fn update_message(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateMessageRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);

    let mut message = storage
        .get_message(&id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::NotFound(format!("Message {} not found", id)))?;

    if let Some(status_str) = req.status {
        let status = match status_str.as_str() {
            "new" => MessageStatus::New,
            "heard" => MessageStatus::Heard,
            "saved" => MessageStatus::Saved,
            "deleted" => MessageStatus::Deleted,
            _ => return Err(ApiError::BadRequest(format!("Invalid status: {}", status_str))),
        };
        message.status = status;
        message.accessed_at = Some(SystemTime::now());
    }

    storage
        .put_message(message.clone())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    info!(message_id = %id, "Updated voicemail message");

    Ok(Json(MessageResponse::from(message)))
}

/// Delete a message.
pub async fn delete_message(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);

    storage
        .delete_message(&id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    info!(message_id = %id, "Deleted voicemail message");

    Ok(StatusCode::NO_CONTENT)
}
