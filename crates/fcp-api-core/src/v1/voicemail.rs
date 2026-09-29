//! Voicemail management API endpoints.

use crate::{error::ApiError, state::ApiState};
use axum::{
    extract::{Multipart, Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use fcp_core::{
    storage::{LineBindingStore, VoicemailStore},
    CredentialStore, GreetingType, LineBindingId, MessageStatus, TenantId, UserId,
    VoicemailGreeting, VoicemailMailbox, VoicemailMessage,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::str::FromStr;
use std::time::SystemTime;
use tracing::{info, warn};

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
    pub retention_days: Option<u32>,
    #[serde(with = "crate::time::epoch_secs_f64")]
    pub created_at: f64,
    #[serde(with = "crate::time::epoch_secs_f64")]
    pub updated_at: f64,
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
            retention_days: mailbox.retention_days,
            created_at: system_time_to_epoch_secs(mailbox.created_at),
            updated_at: system_time_to_epoch_secs(mailbox.updated_at),
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
    #[serde(with = "crate::time::epoch_secs_f64")]
    pub created_at: f64,
    #[serde(default, with = "crate::time::epoch_secs_f64_opt")]
    pub accessed_at: Option<f64>,
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
            created_at: system_time_to_epoch_secs(message.created_at),
            accessed_at: message.accessed_at.map(system_time_to_epoch_secs),
        }
    }
}

/// Create mailbox request.
#[derive(Debug, Deserialize)]
pub struct CreateMailboxRequest {
    pub tenant_id: String,
    pub user_id: String,
    pub line_binding_id: String,
    pub pin: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_max_duration")]
    pub max_message_duration: u32,
    #[serde(default = "default_max_messages")]
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
    /// Double Option: outer = field not provided, inner = null to clear.
    pub retention_days: Option<Option<u32>>,
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

fn ensure_mailbox_in_tenant(
    mailbox: VoicemailMailbox,
    tenant_id: &TenantId,
    id: &str,
) -> Result<VoicemailMailbox, ApiError> {
    if &mailbox.tenant_id == tenant_id {
        Ok(mailbox)
    } else {
        Err(ApiError::NotFound(format!("Mailbox {} not found", id)))
    }
}

async fn get_mailbox_for_tenant(
    state: &ApiState,
    id: &str,
    tenant_id: &TenantId,
) -> Result<VoicemailMailbox, ApiError> {
    let storage = require_storage!(state);
    storage
        .get_mailbox(id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::NotFound(format!("Mailbox {} not found", id)))
        .and_then(|mailbox| ensure_mailbox_in_tenant(mailbox, tenant_id, id))
}

async fn get_message_for_tenant(
    state: &ApiState,
    id: &str,
    tenant_id: &TenantId,
) -> Result<(VoicemailMessage, VoicemailMailbox), ApiError> {
    let storage = require_storage!(state);
    let message = storage
        .get_message(id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::NotFound(format!("Message {} not found", id)))?;
    let mailbox = get_mailbox_for_tenant(state, &message.mailbox_id, tenant_id).await?;
    Ok((message, mailbox))
}

/// List all mailboxes for a tenant.
pub async fn list_mailboxes(
    State(state): State<ApiState>,
    tenant: crate::auth::TenantContext,
    axum::extract::Query(page): axum::extract::Query<crate::page::PageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);

    let tenant_id = tenant.0;

    let mailboxes = storage
        .list_mailboxes(&tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let responses: Vec<MailboxResponse> =
        mailboxes.into_iter().map(MailboxResponse::from).collect();

    Ok(Json(crate::page::paginate(
        responses,
        &page,
        crate::page::Order::Ascending,
        |r| format!("{:>20}/{}", r.extension, r.id),
    )?))
}

/// Get a specific mailbox by ID.
pub async fn get_mailbox(
    State(state): State<ApiState>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let mailbox = get_mailbox_for_tenant(&state, &id, &tenant.0).await?;

    Ok(Json(MailboxResponse::from(mailbox)))
}

/// Validate mailbox control fields.
fn validate_mailbox_controls(max_message_duration: u32, max_messages: u32) -> Result<(), ApiError> {
    if max_message_duration == 0 || max_message_duration > 3600 {
        return Err(ApiError::BadRequest(
            "max_message_duration must be between 1 and 3600 seconds".to_string(),
        ));
    }
    if max_messages == 0 || max_messages > 10000 {
        return Err(ApiError::BadRequest(
            "max_messages must be between 1 and 10000".to_string(),
        ));
    }
    Ok(())
}

/// Create a new voicemail mailbox.
pub async fn create_mailbox(
    State(state): State<ApiState>,
    tenant: crate::auth::TenantContext,
    Json(req): Json<CreateMailboxRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);

    validate_mailbox_controls(req.max_message_duration, req.max_messages)?;

    let user_id = UserId::from_str(&req.user_id)
        .map_err(|e| ApiError::BadRequest(format!("Invalid user_id: {}", e)))?;
    let line_binding_id = LineBindingId::from_str(&req.line_binding_id)
        .map_err(|e| ApiError::BadRequest(format!("Invalid line_binding_id: {}", e)))?;

    // Derive the mailbox extension from the user's username
    let user = storage
        .get_user_by_id(&user_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::NotFound(format!("User {} not found", req.user_id)))?;
    if user.tenant_id != tenant.0 {
        return Err(ApiError::BadRequest(format!(
            "User {} is not in the authenticated tenant",
            req.user_id
        )));
    }
    let line_binding = storage
        .get_line_binding(&line_binding_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| {
            ApiError::NotFound(format!("Line binding {} not found", req.line_binding_id))
        })?;
    if line_binding.tenant_id != tenant.0 {
        return Err(ApiError::BadRequest(format!(
            "Line binding {} is not in the authenticated tenant",
            req.line_binding_id
        )));
    }

    let mut mailbox = VoicemailMailbox::new(
        tenant.0,
        user_id,
        line_binding_id,
        user.username.clone(),
        req.pin,
    );

    mailbox.enabled = req.enabled;
    mailbox.max_message_duration = req.max_message_duration;
    mailbox.max_messages = req.max_messages;

    storage
        .put_mailbox(mailbox.clone())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    info!(
        mailbox_id = %mailbox.id,
        user = %user.username,
        enabled = %mailbox.enabled,
        max_message_duration = %mailbox.max_message_duration,
        max_messages = %mailbox.max_messages,
        "Created voicemail mailbox"
    );

    // Publish mailbox created event so voicemail server can register routes
    state
        .publish_event(fcp_events::PlatformEvent::Voicemail(
            fcp_events::VoicemailEvent::MailboxCreated {
                mailbox_id: mailbox.id.clone(),
                user_id: req.user_id.clone(),
                extension: user.username.clone(),
                tenant_id: tenant.0.to_string(),
                timestamp: SystemTime::now(),
            },
        ))
        .await;

    // Generate TTS default greeting in the background so the API returns immediately
    if let (Some(tts_service), Some(greetings_dir)) = (
        state.tts_service.clone(),
        state.voicemail_greetings_dir.clone(),
    ) {
        let storage = storage.clone();
        let mailbox_id = mailbox.id.clone();
        let display_name = user.display_name.unwrap_or(user.username).clone();

        tokio::spawn(async move {
            let greeting_text = format!(
                "Sorry, {} is not available. Please record your message after the beep.",
                display_name
            );

            match tts_service.synthesize(&greeting_text, None, None).await {
                Ok(tts_path) => {
                    if let Err(e) = tokio::fs::create_dir_all(&greetings_dir).await {
                        warn!(error = %e, "Failed to create voicemail greetings directory");
                        return;
                    }
                    let greeting_filename = format!("{}_unavailable.wav", mailbox_id);
                    let greeting_path = greetings_dir.join(&greeting_filename);
                    match tokio::fs::copy(&tts_path, &greeting_path).await {
                        Ok(_) => {
                            // Re-load mailbox, add greeting, and save
                            match storage.get_mailbox(&mailbox_id).await {
                                Ok(Some(mut mb)) => {
                                    let greeting = VoicemailGreeting::new(
                                        GreetingType::Unavailable,
                                        greeting_path,
                                    );
                                    mb.add_greeting(greeting);
                                    if let Err(e) = storage.put_mailbox(mb).await {
                                        warn!(error = %e, mailbox_id = %mailbox_id, "Failed to save mailbox with TTS greeting");
                                    } else {
                                        info!(mailbox_id = %mailbox_id, "Generated TTS default greeting for mailbox");
                                    }
                                }
                                Ok(None) => {
                                    warn!(mailbox_id = %mailbox_id, "Mailbox deleted before TTS greeting could be saved");
                                }
                                Err(e) => {
                                    warn!(error = %e, mailbox_id = %mailbox_id, "Failed to reload mailbox for TTS greeting");
                                }
                            }
                        }
                        Err(e) => {
                            warn!(error = %e, mailbox_id = %mailbox_id, "Failed to copy TTS greeting file");
                        }
                    }
                }
                Err(e) => {
                    warn!(error = %e, mailbox_id = %mailbox_id, "Failed to generate TTS default greeting");
                }
            }
        });
    }

    Ok((StatusCode::CREATED, Json(MailboxResponse::from(mailbox))))
}

/// Update an existing mailbox.
pub async fn update_mailbox(
    State(state): State<ApiState>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
    Json(req): Json<UpdateMailboxRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);

    let mut mailbox = get_mailbox_for_tenant(&state, &id, &tenant.0).await?;

    // Validate limits if provided
    if let Some(max_duration) = req.max_message_duration {
        if max_duration == 0 || max_duration > 3600 {
            return Err(ApiError::BadRequest(
                "max_message_duration must be between 1 and 3600 seconds".to_string(),
            ));
        }
    }
    if let Some(max_msgs) = req.max_messages {
        if max_msgs == 0 || max_msgs > 10000 {
            return Err(ApiError::BadRequest(
                "max_messages must be between 1 and 10000".to_string(),
            ));
        }
    }

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
    if let Some(rd) = req.retention_days {
        if let Some(days) = rd {
            if days == 0 || days > 365 {
                return Err(ApiError::BadRequest(
                    "retention_days must be between 1 and 365".to_string(),
                ));
            }
        }
        mailbox.retention_days = rd;
    }

    mailbox.updated_at = SystemTime::now();

    storage
        .put_mailbox(mailbox.clone())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    info!(mailbox_id = %id, "Updated voicemail mailbox");

    // Publish mailbox updated event
    state
        .publish_event(fcp_events::PlatformEvent::Voicemail(
            fcp_events::VoicemailEvent::MailboxUpdated {
                mailbox_id: id.clone(),
                timestamp: SystemTime::now(),
            },
        ))
        .await;

    Ok(Json(MailboxResponse::from(mailbox)))
}

/// Delete a mailbox.
pub async fn delete_mailbox(
    State(state): State<ApiState>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);

    get_mailbox_for_tenant(&state, &id, &tenant.0).await?;

    storage
        .delete_mailbox(&id)
        .await
        .map_err(ApiError::storage)?;

    info!(mailbox_id = %id, "Deleted voicemail mailbox");

    // Publish mailbox deleted event so voicemail server can remove routes
    state
        .publish_event(fcp_events::PlatformEvent::Voicemail(
            fcp_events::VoicemailEvent::MailboxDeleted {
                mailbox_id: id.clone(),
                timestamp: SystemTime::now(),
            },
        ))
        .await;

    Ok(StatusCode::NO_CONTENT)
}

/// List messages for a mailbox.
pub async fn list_messages(
    State(state): State<ApiState>,
    tenant: crate::auth::TenantContext,
    Path(mailbox_id): Path<String>,
    axum::extract::Query(page): axum::extract::Query<crate::page::PageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);

    get_mailbox_for_tenant(&state, &mailbox_id, &tenant.0).await?;

    let messages = storage
        .list_messages(&mailbox_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let responses: Vec<MessageResponse> = messages.into_iter().map(MessageResponse::from).collect();

    Ok(Json(crate::page::paginate(
        responses,
        &page,
        crate::page::Order::Descending,
        |r| format!("{:024.6}/{}", r.created_at, r.id),
    )?))
}

/// Get a specific message.
pub async fn get_message(
    State(state): State<ApiState>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let (message, _) = get_message_for_tenant(&state, &id, &tenant.0).await?;

    Ok(Json(MessageResponse::from(message)))
}

/// Update a message (typically to mark as heard/saved).
pub async fn update_message(
    State(state): State<ApiState>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
    Json(req): Json<UpdateMessageRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);

    let (mut message, _) = get_message_for_tenant(&state, &id, &tenant.0).await?;

    if let Some(status_str) = req.status {
        let status = match status_str.as_str() {
            "new" => MessageStatus::New,
            "heard" => MessageStatus::Heard,
            "saved" => MessageStatus::Saved,
            "deleted" => MessageStatus::Deleted,
            _ => {
                return Err(ApiError::BadRequest(format!(
                    "Invalid status: {}",
                    status_str
                )))
            }
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
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);

    get_message_for_tenant(&state, &id, &tenant.0).await?;

    storage
        .delete_message(&id)
        .await
        .map_err(ApiError::storage)?;

    info!(message_id = %id, "Deleted voicemail message");

    Ok(StatusCode::NO_CONTENT)
}

/// Enable a mailbox.
pub async fn enable_mailbox(
    State(state): State<ApiState>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);

    let mut mailbox = get_mailbox_for_tenant(&state, &id, &tenant.0).await?;

    mailbox.enabled = true;
    mailbox.updated_at = SystemTime::now();

    storage
        .put_mailbox(mailbox.clone())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    info!(mailbox_id = %id, "Enabled voicemail mailbox");

    Ok(Json(MailboxResponse::from(mailbox)))
}

/// Disable a mailbox.
pub async fn disable_mailbox(
    State(state): State<ApiState>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);

    let mut mailbox = get_mailbox_for_tenant(&state, &id, &tenant.0).await?;

    mailbox.enabled = false;
    mailbox.updated_at = SystemTime::now();

    storage
        .put_mailbox(mailbox.clone())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    info!(mailbox_id = %id, "Disabled voicemail mailbox");

    Ok(Json(MailboxResponse::from(mailbox)))
}

/// Mailbox statistics response.
#[derive(Debug, Serialize)]
pub struct MailboxStatsResponse {
    pub mailbox_id: String,
    pub total_messages: u32,
    pub new_messages: u32,
    pub heard_messages: u32,
    pub saved_messages: u32,
    pub storage_used: u64,
    pub avg_duration: u32,
}

/// Get mailbox statistics.
pub async fn get_mailbox_stats(
    State(state): State<ApiState>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);

    let _mailbox = get_mailbox_for_tenant(&state, &id, &tenant.0).await?;

    let stats = storage
        .get_mailbox_stats(&id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(MailboxStatsResponse {
        mailbox_id: id,
        total_messages: stats.total_messages,
        new_messages: stats.new_messages,
        heard_messages: stats.heard_messages,
        saved_messages: stats.saved_messages,
        storage_used: stats.storage_used,
        avg_duration: stats.avg_duration,
    }))
}

// --- Greeting Management ---

/// Greeting response.
#[derive(Debug, Serialize)]
pub struct GreetingResponse {
    pub greeting_type: String,
    pub audio_path: String,
    pub is_active: bool,
    /// Unix timestamp in seconds
    #[serde(with = "crate::time::epoch_secs_f64")]
    pub created_at: f64,
    /// Unix timestamp in seconds, or null
    #[serde(default, with = "crate::time::epoch_secs_f64_opt")]
    pub expires_at: Option<f64>,
}

fn system_time_to_epoch_secs(t: SystemTime) -> f64 {
    t.duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

impl From<VoicemailGreeting> for GreetingResponse {
    fn from(g: VoicemailGreeting) -> Self {
        Self {
            greeting_type: g.greeting_type.to_string(),
            audio_path: g.audio_path.to_string_lossy().to_string(),
            is_active: g.is_active,
            created_at: system_time_to_epoch_secs(g.created_at),
            expires_at: g.expires_at.map(system_time_to_epoch_secs),
        }
    }
}

/// Parse a greeting type string into GreetingType.
fn parse_greeting_type(s: &str) -> Result<GreetingType, ApiError> {
    match s.to_lowercase().as_str() {
        "unavailable" => Ok(GreetingType::Unavailable),
        "busy" => Ok(GreetingType::Busy),
        "name" => Ok(GreetingType::Name),
        "temporary" => Ok(GreetingType::Temporary),
        _ => Err(ApiError::BadRequest(format!(
            "Invalid greeting type '{}'. Must be one of: unavailable, busy, name, temporary",
            s
        ))),
    }
}

/// Get the greetings directory or return an error.
fn require_greetings_dir(state: &ApiState) -> Result<&PathBuf, ApiError> {
    state.voicemail_greetings_dir.as_ref().ok_or_else(|| {
        ApiError::Internal("Voicemail greetings directory is not configured".to_string())
    })
}

/// List all greetings for a mailbox.
pub async fn list_greetings(
    State(state): State<ApiState>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
    axum::extract::Query(page): axum::extract::Query<crate::page::PageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let mailbox = get_mailbox_for_tenant(&state, &id, &tenant.0).await?;

    let responses: Vec<GreetingResponse> = mailbox
        .greetings
        .into_iter()
        .map(GreetingResponse::from)
        .collect();

    Ok(Json(crate::page::paginate(
        responses,
        &page,
        crate::page::Order::Ascending,
        |r| r.greeting_type.clone(),
    )?))
}

/// Upload a greeting (multipart: greeting_type + audio file).
pub async fn upload_greeting(
    State(state): State<ApiState>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
    mut multipart: Multipart,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);
    let greetings_dir = require_greetings_dir(&state)?;

    // Parse multipart fields
    let mut greeting_type_str: Option<String> = None;
    let mut audio_data: Option<Vec<u8>> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::BadRequest(format!("Failed to read multipart field: {}", e)))?
    {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "greeting_type" => {
                let text = field.text().await.map_err(|e| {
                    ApiError::BadRequest(format!("Failed to read greeting_type: {}", e))
                })?;
                greeting_type_str = Some(text);
            }
            "audio" => {
                let bytes = field.bytes().await.map_err(|e| {
                    ApiError::BadRequest(format!("Failed to read audio data: {}", e))
                })?;
                audio_data = Some(bytes.to_vec());
            }
            _ => {} // ignore unknown fields
        }
    }

    let greeting_type_str = greeting_type_str
        .ok_or_else(|| ApiError::BadRequest("Missing 'greeting_type' field".to_string()))?;
    let audio_data =
        audio_data.ok_or_else(|| ApiError::BadRequest("Missing 'audio' file field".to_string()))?;
    if audio_data.is_empty() {
        return Err(ApiError::BadRequest("Audio file is empty".to_string()));
    }

    let greeting_type = parse_greeting_type(&greeting_type_str)?;

    // Load mailbox
    let mut mailbox = get_mailbox_for_tenant(&state, &id, &tenant.0).await?;

    // Create directory for this mailbox's greetings
    let mailbox_dir = greetings_dir.join(&id);
    tokio::fs::create_dir_all(&mailbox_dir)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to create greetings directory: {}", e)))?;

    // Write audio file
    let file_path = mailbox_dir.join(format!("{}.wav", greeting_type));
    tokio::fs::write(&file_path, &audio_data)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to write greeting file: {}", e)))?;

    // Add greeting to mailbox and save
    let greeting = VoicemailGreeting::new(greeting_type, file_path);
    mailbox.add_greeting(greeting.clone());

    storage
        .put_mailbox(mailbox)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    info!(
        mailbox_id = %id,
        greeting_type = %greeting_type_str,
        "Uploaded voicemail greeting"
    );

    Ok((StatusCode::CREATED, Json(GreetingResponse::from(greeting))))
}

/// Delete a greeting.
pub async fn delete_greeting(
    State(state): State<ApiState>,
    tenant: crate::auth::TenantContext,
    Path((id, greeting_type_str)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);

    let greeting_type = parse_greeting_type(&greeting_type_str)?;

    let mut mailbox = get_mailbox_for_tenant(&state, &id, &tenant.0).await?;

    // Find and remove the greeting
    let had_greeting = mailbox
        .greetings
        .iter()
        .any(|g| g.greeting_type == greeting_type);
    if !had_greeting {
        return Err(ApiError::NotFound(format!(
            "Greeting '{}' not found for mailbox {}",
            greeting_type_str, id
        )));
    }

    // Try to remove the audio file (best-effort)
    if let Some(g) = mailbox
        .greetings
        .iter()
        .find(|g| g.greeting_type == greeting_type)
    {
        let _ = tokio::fs::remove_file(&g.audio_path).await;
    }

    mailbox
        .greetings
        .retain(|g| g.greeting_type != greeting_type);
    mailbox.updated_at = SystemTime::now();

    storage
        .put_mailbox(mailbox)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    info!(
        mailbox_id = %id,
        greeting_type = %greeting_type_str,
        "Deleted voicemail greeting"
    );

    Ok(StatusCode::NO_CONTENT)
}

/// Request body for toggling greeting active state.
#[derive(Debug, Deserialize)]
pub struct SetGreetingActiveRequest {
    pub active: bool,
}

/// Set the active state of a greeting.
pub async fn set_greeting_active(
    State(state): State<ApiState>,
    tenant: crate::auth::TenantContext,
    Path((id, greeting_type_str)): Path<(String, String)>,
    Json(req): Json<SetGreetingActiveRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);

    let greeting_type = parse_greeting_type(&greeting_type_str)?;

    let mut mailbox = get_mailbox_for_tenant(&state, &id, &tenant.0).await?;

    if !mailbox
        .greetings
        .iter()
        .any(|g| g.greeting_type == greeting_type)
    {
        return Err(ApiError::NotFound(format!(
            "Greeting '{}' not found for mailbox {}",
            greeting_type_str, id
        )));
    }

    mailbox.set_greeting_active(greeting_type, req.active);

    storage
        .put_mailbox(mailbox)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    info!(
        mailbox_id = %id,
        greeting_type = %greeting_type_str,
        active = %req.active,
        "Updated greeting active state"
    );

    Ok(Json(serde_json::json!({
        "status": "ok",
        "greeting_type": greeting_type_str,
        "is_active": req.active,
    })))
}

/// Download a greeting audio file.
pub async fn download_greeting_audio(
    State(state): State<ApiState>,
    tenant: crate::auth::TenantContext,
    Path((id, greeting_type_str)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let greeting_type = parse_greeting_type(&greeting_type_str)?;

    let mailbox = get_mailbox_for_tenant(&state, &id, &tenant.0).await?;

    let greeting = mailbox
        .greetings
        .iter()
        .find(|g| g.greeting_type == greeting_type)
        .ok_or_else(|| {
            ApiError::NotFound(format!(
                "Greeting '{}' not found for mailbox {}",
                greeting_type_str, id
            ))
        })?;

    let data = tokio::fs::read(&greeting.audio_path)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to read greeting file: {}", e)))?;

    Ok((
        StatusCode::OK,
        [
            (axum::http::header::CONTENT_TYPE, "audio/wav"),
            (axum::http::header::CONTENT_DISPOSITION, "inline"),
        ],
        data,
    ))
}
