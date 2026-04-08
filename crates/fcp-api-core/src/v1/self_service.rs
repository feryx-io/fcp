//! Self-service API endpoints for end users.
//!
//! All endpoints are scoped to the authenticated user via JWT claims.
//! Route prefix: `/api/v1/me/*`

use crate::error::ApiError;
use crate::state::ApiState;
use crate::v1::auth_session::{extract_claims, extract_claims_from_headers, Claims};
use axum::{
    extract::{Multipart, Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use fcp_core::{
    storage::{password, ConferenceStore, LineBindingStore, PhoneNumberStore, VoicemailStore},
    CredentialStore, GreetingType, MessageStatus, UserId, VoicemailGreeting,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::str::FromStr;
use std::time::SystemTime;
use tracing::info;

/// Helper macro to get storage or return error.
macro_rules! require_storage {
    ($state:expr) => {
        $state
            .storage
            .as_ref()
            .ok_or_else(|| ApiError::Internal("Storage not configured".to_string()))?
    };
}

async fn load_claims_user(
    storage: &std::sync::Arc<fcp_core::DynStorage>,
    claims: &Claims,
    state: &ApiState,
) -> Result<fcp_core::User, ApiError> {
    let tenant_id = resolve_tenant_from_claims(claims, state);
    storage
        .get_user_in_tenant(&claims.sub, &tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::NotFound("User not found".to_string()))
}

// ---- Profile Endpoints ----

/// Profile response (limited fields for end users).
#[derive(Debug, Serialize)]
pub struct ProfileResponse {
    pub username: String,
    pub display_name: Option<String>,
    pub email: Option<String>,
    pub phone_number: Option<String>,
    pub extension: Option<String>,
    pub department: Option<String>,
    pub job_title: Option<String>,
    pub location: Option<String>,
    pub language: String,
    pub timezone: String,
    pub role: String,
    pub has_profile_picture: bool,
}

/// GET /me/profile
pub async fn get_profile(
    State(state): State<ApiState>,
    req: axum::extract::Request,
) -> Result<impl IntoResponse, ApiError> {
    let claims = extract_claims(&req, &state)?;
    let storage = require_storage!(state);

    let user = load_claims_user(storage, &claims, &state).await?;

    let has_profile_picture = user.profile_picture.is_some();
    Ok(Json(ProfileResponse {
        username: user.username,
        display_name: user.display_name,
        email: user.email,
        phone_number: user.phone_number,
        extension: user.extension,
        department: user.department,
        job_title: user.job_title,
        location: user.location,
        language: user.language,
        timezone: user.timezone,
        role: format!("{:?}", user.role).to_lowercase(),
        has_profile_picture,
    }))
}

#[derive(Debug, Deserialize)]
pub struct UpdateProfileRequest {
    pub display_name: Option<String>,
    pub email: Option<String>,
    pub phone_number: Option<String>,
    pub timezone: Option<String>,
    pub language: Option<String>,
}

/// PUT /me/profile
pub async fn update_profile(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(body): Json<UpdateProfileRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let claims = extract_claims_from_headers(&headers, &state)?;
    let storage = require_storage!(state);

    let mut user = load_claims_user(storage, &claims, &state).await?;

    if let Some(display_name) = body.display_name {
        user.display_name = Some(display_name);
    }
    if let Some(email) = body.email {
        user.email = Some(email);
    }
    if let Some(phone_number) = body.phone_number {
        user.phone_number = Some(phone_number);
    }
    if let Some(timezone) = body.timezone {
        user.timezone = timezone;
    }
    if let Some(language) = body.language {
        user.language = language;
    }
    user.updated_at = SystemTime::now();

    storage
        .put_user(user.clone())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    info!(username = %claims.sub, "User updated own profile");

    let has_profile_picture = user.profile_picture.is_some();
    Ok(Json(ProfileResponse {
        username: user.username,
        display_name: user.display_name,
        email: user.email,
        phone_number: user.phone_number,
        extension: user.extension,
        department: user.department,
        job_title: user.job_title,
        location: user.location,
        language: user.language,
        timezone: user.timezone,
        role: format!("{:?}", user.role).to_lowercase(),
        has_profile_picture,
    }))
}

#[derive(Debug, Deserialize)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

/// PUT /me/password
pub async fn change_password(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(body): Json<ChangePasswordRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let claims = extract_claims_from_headers(&headers, &state)?;
    let storage = require_storage!(state);

    let mut user = load_claims_user(storage, &claims, &state).await?;

    // Verify current password
    let valid = password::verify_password(&body.current_password, &user.password)
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !valid {
        return Err(ApiError::Unauthorized(
            "Current password is incorrect".to_string(),
        ));
    }

    // Hash new password
    let password_hash = password::hash_password(&body.new_password)
        .map_err(|e| ApiError::BadRequest(format!("Password validation failed: {}", e)))?;

    // Compute SIP HA1 hash
    let sip_ha1_hash = password::compute_ha1(&claims.sub, &state.realm, &body.new_password);

    user.password = password_hash;
    user.sip_ha1_hash = Some(sip_ha1_hash);
    user.password_last_reset = Some(SystemTime::now());
    user.updated_at = SystemTime::now();

    storage
        .put_user(user)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    info!(username = %claims.sub, "User changed own password");

    Ok(StatusCode::NO_CONTENT)
}

// ---- Voicemail Endpoints ----

/// Mailbox response (reuse same shape as admin).
#[derive(Debug, Serialize)]
pub struct MailboxResponse {
    pub id: String,
    pub extension: String,
    pub enabled: bool,
    pub max_message_duration: u32,
    pub max_messages: u32,
    pub email_notification: bool,
    pub email_address: Option<String>,
    pub attach_audio: bool,
    pub retention_days: Option<u32>,
}

/// GET /me/voicemail
pub async fn get_voicemail(
    State(state): State<ApiState>,
    req: axum::extract::Request,
) -> Result<impl IntoResponse, ApiError> {
    let claims = extract_claims(&req, &state)?;
    let storage = require_storage!(state);
    let (mailbox, _user_id) = get_user_mailbox(
        &claims.sub,
        &resolve_tenant_from_claims(&claims, &state),
        storage,
    )
    .await?;

    Ok(Json(MailboxResponse {
        id: mailbox.id,
        extension: mailbox.extension,
        enabled: mailbox.enabled,
        max_message_duration: mailbox.max_message_duration,
        max_messages: mailbox.max_messages,
        email_notification: mailbox.email_notification,
        email_address: mailbox.email_address,
        attach_audio: mailbox.attach_audio,
        retention_days: mailbox.retention_days,
    }))
}

#[derive(Debug, Deserialize)]
pub struct UpdateVoicemailRequest {
    pub pin: Option<String>,
    pub email_notification: Option<bool>,
    pub email_address: Option<String>,
    pub attach_audio: Option<bool>,
}

/// PUT /me/voicemail
pub async fn update_voicemail(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(body): Json<UpdateVoicemailRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let claims = extract_claims_from_headers(&headers, &state)?;
    let storage = require_storage!(state);
    let (mut mailbox, _user_id) = get_user_mailbox(
        &claims.sub,
        &resolve_tenant_from_claims(&claims, &state),
        storage,
    )
    .await?;

    if let Some(pin) = body.pin {
        mailbox.pin = pin;
    }
    if let Some(email_notification) = body.email_notification {
        mailbox.email_notification = email_notification;
    }
    if let Some(email_address) = body.email_address {
        mailbox.email_address = Some(email_address);
    }
    if let Some(attach_audio) = body.attach_audio {
        mailbox.attach_audio = attach_audio;
    }
    mailbox.updated_at = SystemTime::now();

    storage
        .put_mailbox(mailbox.clone())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    info!(username = %claims.sub, "User updated own voicemail settings");

    Ok(Json(MailboxResponse {
        id: mailbox.id,
        extension: mailbox.extension,
        enabled: mailbox.enabled,
        max_message_duration: mailbox.max_message_duration,
        max_messages: mailbox.max_messages,
        email_notification: mailbox.email_notification,
        email_address: mailbox.email_address,
        attach_audio: mailbox.attach_audio,
        retention_days: mailbox.retention_days,
    }))
}

/// Mailbox stats response.
#[derive(Debug, Serialize)]
pub struct MailboxStatsResponse {
    pub mailbox_id: String,
    pub total_messages: usize,
    pub new_messages: usize,
    pub heard_messages: usize,
    pub saved_messages: usize,
}

/// GET /me/voicemail/stats
pub async fn get_voicemail_stats(
    State(state): State<ApiState>,
    req: axum::extract::Request,
) -> Result<impl IntoResponse, ApiError> {
    let claims = extract_claims(&req, &state)?;
    let storage = require_storage!(state);
    let (mailbox, _user_id) = get_user_mailbox(
        &claims.sub,
        &resolve_tenant_from_claims(&claims, &state),
        storage,
    )
    .await?;

    let messages = storage
        .list_messages(&mailbox.id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let total = messages.len();
    let new_count = messages
        .iter()
        .filter(|m| m.status == MessageStatus::New)
        .count();
    let heard = messages
        .iter()
        .filter(|m| m.status == MessageStatus::Heard)
        .count();
    let saved = messages
        .iter()
        .filter(|m| m.status == MessageStatus::Saved)
        .count();

    Ok(Json(MailboxStatsResponse {
        mailbox_id: mailbox.id,
        total_messages: total,
        new_messages: new_count,
        heard_messages: heard,
        saved_messages: saved,
    }))
}

/// Message response.
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
    pub created_at: f64,
    pub accessed_at: Option<f64>,
}

/// GET /me/voicemail/messages
pub async fn list_voicemail_messages(
    State(state): State<ApiState>,
    req: axum::extract::Request,
) -> Result<impl IntoResponse, ApiError> {
    let claims = extract_claims(&req, &state)?;
    let storage = require_storage!(state);
    let (mailbox, _user_id) = get_user_mailbox(
        &claims.sub,
        &resolve_tenant_from_claims(&claims, &state),
        storage,
    )
    .await?;

    let messages = storage
        .list_messages(&mailbox.id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let responses: Vec<MessageResponse> = messages.into_iter().map(message_to_response).collect();
    Ok(Json(responses))
}

#[derive(Debug, Deserialize)]
pub struct UpdateMessageRequest {
    pub status: Option<String>,
}

/// PUT /me/voicemail/messages/:id
pub async fn update_voicemail_message(
    State(state): State<ApiState>,
    Path(message_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<UpdateMessageRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let claims = extract_claims_from_headers(&headers, &state)?;
    let storage = require_storage!(state);
    let (mailbox, _user_id) = get_user_mailbox(
        &claims.sub,
        &resolve_tenant_from_claims(&claims, &state),
        storage,
    )
    .await?;

    let mut message = storage
        .get_message(&message_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::NotFound("Message not found".to_string()))?;

    // Verify ownership
    if message.mailbox_id != mailbox.id {
        return Err(ApiError::Forbidden(
            "Message does not belong to your mailbox".to_string(),
        ));
    }

    if let Some(status_str) = body.status {
        message.status = match status_str.as_str() {
            "heard" => MessageStatus::Heard,
            "saved" => MessageStatus::Saved,
            "new" => MessageStatus::New,
            _ => {
                return Err(ApiError::BadRequest(format!(
                    "Invalid status '{}'. Must be: new, heard, saved",
                    status_str
                )));
            }
        };
        message.accessed_at = Some(SystemTime::now());
    }

    storage
        .put_message(message.clone())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(message_to_response(message)))
}

/// DELETE /me/voicemail/messages/:id
pub async fn delete_voicemail_message(
    State(state): State<ApiState>,
    Path(message_id): Path<String>,
    req: axum::extract::Request,
) -> Result<impl IntoResponse, ApiError> {
    let claims = extract_claims(&req, &state)?;
    let storage = require_storage!(state);
    let (mailbox, _user_id) = get_user_mailbox(
        &claims.sub,
        &resolve_tenant_from_claims(&claims, &state),
        storage,
    )
    .await?;

    let message = storage
        .get_message(&message_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::NotFound("Message not found".to_string()))?;

    // Verify ownership
    if message.mailbox_id != mailbox.id {
        return Err(ApiError::Forbidden(
            "Message does not belong to your mailbox".to_string(),
        ));
    }

    storage
        .delete_message(&message_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    info!(username = %claims.sub, message_id = %message_id, "User deleted voicemail message");

    Ok(StatusCode::NO_CONTENT)
}

/// GET /me/voicemail/messages/:id/audio
pub async fn get_message_audio(
    State(state): State<ApiState>,
    Path(message_id): Path<String>,
    req: axum::extract::Request,
) -> Result<impl IntoResponse, ApiError> {
    let claims = extract_claims(&req, &state)?;
    let storage = require_storage!(state);
    let (mailbox, _user_id) = get_user_mailbox(
        &claims.sub,
        &resolve_tenant_from_claims(&claims, &state),
        storage,
    )
    .await?;

    let message = storage
        .get_message(&message_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::NotFound("Message not found".to_string()))?;

    // Verify ownership
    if message.mailbox_id != mailbox.id {
        return Err(ApiError::Forbidden(
            "Message does not belong to your mailbox".to_string(),
        ));
    }

    let data = tokio::fs::read(&message.recording_path)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to read recording file: {}", e)))?;

    Ok((
        StatusCode::OK,
        [
            (axum::http::header::CONTENT_TYPE, "audio/wav"),
            (axum::http::header::CONTENT_DISPOSITION, "inline"),
        ],
        data,
    ))
}

// ---- Voicemail Greetings Endpoints ----

/// Greeting response.
#[derive(Debug, Serialize)]
pub struct GreetingResponse {
    pub greeting_type: String,
    pub is_active: bool,
    pub created_at: f64,
    pub expires_at: Option<f64>,
}

/// GET /me/voicemail/greetings
pub async fn list_voicemail_greetings(
    State(state): State<ApiState>,
    req: axum::extract::Request,
) -> Result<impl IntoResponse, ApiError> {
    let claims = extract_claims(&req, &state)?;
    let storage = require_storage!(state);
    let (mailbox, _user_id) = get_user_mailbox(
        &claims.sub,
        &resolve_tenant_from_claims(&claims, &state),
        storage,
    )
    .await?;

    let responses: Vec<GreetingResponse> = mailbox
        .greetings
        .into_iter()
        .map(|g| GreetingResponse {
            greeting_type: g.greeting_type.to_string(),
            is_active: g.is_active,
            created_at: system_time_to_epoch_secs(g.created_at),
            expires_at: g.expires_at.map(system_time_to_epoch_secs),
        })
        .collect();

    Ok(Json(responses))
}

/// POST /me/voicemail/greetings
pub async fn upload_voicemail_greeting(
    State(state): State<ApiState>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Result<impl IntoResponse, ApiError> {
    let claims = extract_claims_from_headers(&headers, &state)?;
    let storage = require_storage!(state);
    let greetings_dir = require_greetings_dir(&state)?;
    let (mut mailbox, _user_id) = get_user_mailbox(
        &claims.sub,
        &resolve_tenant_from_claims(&claims, &state),
        storage,
    )
    .await?;

    // Parse multipart fields
    let mut greeting_type_str: Option<String> = None;
    let mut audio_data: Option<Vec<u8>> = None;
    let mut expires_in_str: Option<String> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::BadRequest(format!("Failed to read multipart field: {}", e)))?
    {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "greeting_type" => {
                greeting_type_str = Some(field.text().await.map_err(|e| {
                    ApiError::BadRequest(format!("Failed to read greeting_type: {}", e))
                })?);
            }
            "audio" => {
                let bytes = field.bytes().await.map_err(|e| {
                    ApiError::BadRequest(format!("Failed to read audio data: {}", e))
                })?;
                audio_data = Some(bytes.to_vec());
            }
            "expires_in" => {
                expires_in_str = Some(field.text().await.map_err(|e| {
                    ApiError::BadRequest(format!("Failed to read expires_in: {}", e))
                })?);
            }
            _ => {}
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

    // Create directory for this mailbox's greetings
    let mailbox_dir = greetings_dir.join(&mailbox.id);
    tokio::fs::create_dir_all(&mailbox_dir)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to create greetings directory: {}", e)))?;

    // Write audio file
    let file_path = mailbox_dir.join(format!("{}.wav", greeting_type));
    tokio::fs::write(&file_path, &audio_data)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to write greeting file: {}", e)))?;

    // Create greeting (with optional expiration for temporary greetings)
    let greeting = if let Some(expires_in) = expires_in_str {
        let hours: u64 = expires_in.parse().map_err(|_| {
            ApiError::BadRequest("expires_in must be a number of hours".to_string())
        })?;
        let expires_at = SystemTime::now() + std::time::Duration::from_secs(hours * 3600);
        VoicemailGreeting::with_expiration(greeting_type, file_path, expires_at)
    } else {
        VoicemailGreeting::new(greeting_type, file_path)
    };

    mailbox.add_greeting(greeting.clone());

    storage
        .put_mailbox(mailbox)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    info!(username = %claims.sub, greeting_type = %greeting_type_str, "User uploaded voicemail greeting");

    Ok((
        StatusCode::CREATED,
        Json(GreetingResponse {
            greeting_type: greeting.greeting_type.to_string(),
            is_active: greeting.is_active,
            created_at: system_time_to_epoch_secs(greeting.created_at),
            expires_at: greeting.expires_at.map(system_time_to_epoch_secs),
        }),
    ))
}

/// POST /me/voicemail/greetings/synthesize
///
/// Synthesize a greeting from text using TTS.
pub async fn synthesize_voicemail_greeting(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(body): Json<SynthesizeGreetingRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let claims = extract_claims_from_headers(&headers, &state)?;
    let storage = require_storage!(state);
    let greetings_dir = require_greetings_dir(&state)?;
    let (mut mailbox, _user_id) = get_user_mailbox(
        &claims.sub,
        &resolve_tenant_from_claims(&claims, &state),
        storage,
    )
    .await?;

    let tts_service = state
        .tts_service
        .as_ref()
        .ok_or_else(|| ApiError::Internal("TTS service is not configured".to_string()))?;

    let greeting_type = parse_greeting_type(&body.greeting_type)?;

    if body.text.trim().is_empty() {
        return Err(ApiError::BadRequest("Text must not be empty".to_string()));
    }

    let language = body.language.as_deref().unwrap_or("en-US");
    let voice = body.voice.as_deref();

    // Synthesize audio via TTS
    let cached_path = tts_service
        .synthesize(&body.text, Some(language), voice)
        .await
        .map_err(|e| ApiError::Internal(format!("TTS synthesis failed: {}", e)))?;

    // Copy to greeting file location
    let mailbox_dir = greetings_dir.join(&mailbox.id);
    tokio::fs::create_dir_all(&mailbox_dir)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to create greetings directory: {}", e)))?;

    let file_path = mailbox_dir.join(format!("{}.wav", greeting_type));
    tokio::fs::copy(&cached_path, &file_path)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to copy synthesized audio: {}", e)))?;

    // Create greeting (with optional expiration for temporary greetings)
    let greeting = if let Some(hours) = body.expires_in_hours {
        let expires_at = SystemTime::now() + std::time::Duration::from_secs(hours * 3600);
        VoicemailGreeting::with_expiration(greeting_type, file_path, expires_at)
    } else {
        VoicemailGreeting::new(greeting_type, file_path)
    };

    mailbox.add_greeting(greeting.clone());

    storage
        .put_mailbox(mailbox)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    info!(
        username = %claims.sub,
        greeting_type = %body.greeting_type,
        voice = ?voice,
        "User synthesized voicemail greeting via TTS"
    );

    Ok((
        StatusCode::CREATED,
        Json(GreetingResponse {
            greeting_type: greeting.greeting_type.to_string(),
            is_active: greeting.is_active,
            created_at: system_time_to_epoch_secs(greeting.created_at),
            expires_at: greeting.expires_at.map(system_time_to_epoch_secs),
        }),
    ))
}

#[derive(Debug, Deserialize)]
pub struct SynthesizeGreetingRequest {
    pub greeting_type: String,
    pub text: String,
    pub language: Option<String>,
    pub voice: Option<String>,
    pub expires_in_hours: Option<u64>,
}

/// DELETE /me/voicemail/greetings/:type
pub async fn delete_voicemail_greeting(
    State(state): State<ApiState>,
    Path(greeting_type_str): Path<String>,
    req: axum::extract::Request,
) -> Result<impl IntoResponse, ApiError> {
    let claims = extract_claims(&req, &state)?;
    let storage = require_storage!(state);
    let (mut mailbox, _user_id) = get_user_mailbox(
        &claims.sub,
        &resolve_tenant_from_claims(&claims, &state),
        storage,
    )
    .await?;

    let greeting_type = parse_greeting_type(&greeting_type_str)?;

    let had_greeting = mailbox
        .greetings
        .iter()
        .any(|g| g.greeting_type == greeting_type);
    if !had_greeting {
        return Err(ApiError::NotFound(format!(
            "Greeting '{}' not found",
            greeting_type_str
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

    info!(username = %claims.sub, greeting_type = %greeting_type_str, "User deleted voicemail greeting");

    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
pub struct SetGreetingActiveRequest {
    pub active: bool,
}

/// PUT /me/voicemail/greetings/:type/active
pub async fn set_voicemail_greeting_active(
    State(state): State<ApiState>,
    Path(greeting_type_str): Path<String>,
    headers: HeaderMap,
    Json(body): Json<SetGreetingActiveRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let claims = extract_claims_from_headers(&headers, &state)?;
    let storage = require_storage!(state);
    let (mut mailbox, _user_id) = get_user_mailbox(
        &claims.sub,
        &resolve_tenant_from_claims(&claims, &state),
        storage,
    )
    .await?;

    let greeting_type = parse_greeting_type(&greeting_type_str)?;

    if !mailbox
        .greetings
        .iter()
        .any(|g| g.greeting_type == greeting_type)
    {
        return Err(ApiError::NotFound(format!(
            "Greeting '{}' not found",
            greeting_type_str
        )));
    }

    mailbox.set_greeting_active(greeting_type, body.active);

    storage
        .put_mailbox(mailbox)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(serde_json::json!({
        "greeting_type": greeting_type_str,
        "is_active": body.active,
    })))
}

/// GET /me/voicemail/greetings/:type/audio
pub async fn download_voicemail_greeting_audio(
    State(state): State<ApiState>,
    Path(greeting_type_str): Path<String>,
    req: axum::extract::Request,
) -> Result<impl IntoResponse, ApiError> {
    let claims = extract_claims(&req, &state)?;
    let storage = require_storage!(state);
    let (mailbox, _user_id) = get_user_mailbox(
        &claims.sub,
        &resolve_tenant_from_claims(&claims, &state),
        storage,
    )
    .await?;

    let greeting_type = parse_greeting_type(&greeting_type_str)?;

    let greeting = mailbox
        .greetings
        .iter()
        .find(|g| g.greeting_type == greeting_type)
        .ok_or_else(|| ApiError::NotFound(format!("Greeting '{}' not found", greeting_type_str)))?;

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

// ---- Call Forwarding Endpoints ----

/// Forwarding response (line binding subset).
#[derive(Debug, Serialize)]
pub struct ForwardingResponse {
    pub id: String,
    pub phone_number_id: String,
    pub phone_number: Option<String>,
    pub forward_unconditional: Option<String>,
    pub forward_on_busy: Option<String>,
    pub forward_on_no_answer: Option<String>,
    pub no_answer_timeout_secs: u64,
}

/// GET /me/forwarding
pub async fn get_forwarding(
    State(state): State<ApiState>,
    req: axum::extract::Request,
) -> Result<impl IntoResponse, ApiError> {
    let claims = extract_claims(&req, &state)?;
    let storage = require_storage!(state);

    let user = load_claims_user(storage, &claims, &state).await?;

    let bindings = storage
        .list_line_bindings_by_user(&user.id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let mut responses = Vec::with_capacity(bindings.len());
    for b in bindings {
        // Look up the actual phone number (E.164) from the ID
        let phone_number = storage
            .get_phone_number(&b.phone_number_id)
            .await
            .ok()
            .flatten()
            .map(|pn| pn.e164);

        responses.push(ForwardingResponse {
            id: b.id.to_string(),
            phone_number_id: b.phone_number_id.to_string(),
            phone_number,
            forward_unconditional: b.forward_unconditional,
            forward_on_busy: b.forward_on_busy,
            forward_on_no_answer: b.forward_on_no_answer,
            no_answer_timeout_secs: b.no_answer_timeout.as_secs(),
        });
    }

    Ok(Json(responses))
}

#[derive(Debug, Deserialize)]
pub struct UpdateForwardingRequest {
    pub forward_unconditional: Option<String>,
    pub forward_on_busy: Option<String>,
    pub forward_on_no_answer: Option<String>,
    pub no_answer_timeout_secs: Option<u64>,
}

/// PATCH /me/forwarding/:id
pub async fn update_forwarding(
    State(state): State<ApiState>,
    Path(binding_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<UpdateForwardingRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let claims = extract_claims_from_headers(&headers, &state)?;
    let storage = require_storage!(state);

    let user = load_claims_user(storage, &claims, &state).await?;

    let lb_id = fcp_core::LineBindingId::from_str(&binding_id)
        .map_err(|e| ApiError::BadRequest(format!("Invalid binding ID: {}", e)))?;

    let mut binding = storage
        .get_line_binding(&lb_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::NotFound("Line binding not found".to_string()))?;

    // Verify ownership
    if binding.owner_user_id.as_ref() != Some(&user.id) {
        return Err(ApiError::Forbidden(
            "Line binding does not belong to you".to_string(),
        ));
    }

    // Update only forwarding fields
    if let Some(fwd) = body.forward_unconditional {
        binding.forward_unconditional = if fwd.is_empty() { None } else { Some(fwd) };
    }
    if let Some(fwd) = body.forward_on_busy {
        binding.forward_on_busy = if fwd.is_empty() { None } else { Some(fwd) };
    }
    if let Some(fwd) = body.forward_on_no_answer {
        binding.forward_on_no_answer = if fwd.is_empty() { None } else { Some(fwd) };
    }
    if let Some(timeout) = body.no_answer_timeout_secs {
        binding.no_answer_timeout = std::time::Duration::from_secs(timeout);
    }
    binding.updated_at = SystemTime::now();

    storage
        .put_line_binding(binding.clone())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    info!(username = %claims.sub, binding_id = %binding_id, "User updated call forwarding");

    // Look up the actual phone number (E.164) from the ID
    let phone_number = storage
        .get_phone_number(&binding.phone_number_id)
        .await
        .ok()
        .flatten()
        .map(|pn| pn.e164);

    Ok(Json(ForwardingResponse {
        id: binding.id.to_string(),
        phone_number_id: binding.phone_number_id.to_string(),
        phone_number,
        forward_unconditional: binding.forward_unconditional,
        forward_on_busy: binding.forward_on_busy,
        forward_on_no_answer: binding.forward_on_no_answer,
        no_answer_timeout_secs: binding.no_answer_timeout.as_secs(),
    }))
}

// ---- Helper Functions ----

/// Get the authenticated user's mailbox, verifying ownership.
async fn get_user_mailbox(
    username: &str,
    tenant_id: &fcp_core::TenantId,
    storage: &std::sync::Arc<fcp_core::DynStorage>,
) -> Result<(fcp_core::VoicemailMailbox, UserId), ApiError> {
    let user = storage
        .get_user_in_tenant(username, tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::NotFound("User not found".to_string()))?;

    let mailbox = storage
        .get_mailbox_by_user(&user.id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| {
            ApiError::NotFound("No voicemail mailbox found for your account".to_string())
        })?;

    Ok((mailbox, user.id))
}

/// Parse a greeting type string.
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

// ---- Profile Picture (Avatar) Endpoints ----

/// Get the profile pictures directory or return an error.
fn require_profile_pictures_dir(state: &ApiState) -> Result<&PathBuf, ApiError> {
    state.profile_pictures_dir.as_ref().ok_or_else(|| {
        ApiError::Internal("Profile pictures directory is not configured".to_string())
    })
}

const MAX_AVATAR_SIZE: usize = 2 * 1024 * 1024; // 2 MB

/// POST /me/profile/avatar
pub async fn upload_avatar(
    State(state): State<ApiState>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Result<impl IntoResponse, ApiError> {
    let claims = extract_claims_from_headers(&headers, &state)?;
    let storage = require_storage!(state);
    let pictures_dir = require_profile_pictures_dir(&state)?;

    let mut user = load_claims_user(storage, &claims, &state).await?;

    let mut image_data: Option<Vec<u8>> = None;
    let mut content_type: Option<String> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::BadRequest(format!("Failed to read multipart field: {}", e)))?
    {
        let name = field.name().unwrap_or("").to_string();
        if name == "avatar" || name == "file" {
            content_type = field.content_type().map(|s| s.to_string());
            let bytes = field
                .bytes()
                .await
                .map_err(|e| ApiError::BadRequest(format!("Failed to read file data: {}", e)))?;
            if bytes.len() > MAX_AVATAR_SIZE {
                return Err(ApiError::BadRequest(format!(
                    "File too large ({} bytes). Maximum is {} bytes.",
                    bytes.len(),
                    MAX_AVATAR_SIZE
                )));
            }
            image_data = Some(bytes.to_vec());
        }
    }

    let image_data = image_data
        .ok_or_else(|| ApiError::BadRequest("Missing 'avatar' file field".to_string()))?;
    if image_data.is_empty() {
        return Err(ApiError::BadRequest("Image file is empty".to_string()));
    }

    // Determine extension from content type
    let ext = match content_type.as_deref() {
        Some("image/jpeg") | Some("image/jpg") => "jpg",
        Some("image/png") => "png",
        Some("image/webp") => "webp",
        _ => {
            return Err(ApiError::BadRequest(
                "Unsupported image type. Must be JPEG, PNG, or WebP.".to_string(),
            ));
        }
    };

    // Create directory if needed
    tokio::fs::create_dir_all(pictures_dir)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to create pictures directory: {}", e)))?;

    // Remove old avatar if it exists (different extension)
    if let Some(ref old_filename) = user.profile_picture {
        let old_path = pictures_dir.join(old_filename);
        let _ = tokio::fs::remove_file(&old_path).await;
    }

    // Write new file
    let filename = format!("{}.{}", user.id, ext);
    let file_path = pictures_dir.join(&filename);
    tokio::fs::write(&file_path, &image_data)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to write avatar file: {}", e)))?;

    // Update user record
    user.profile_picture = Some(filename);
    user.updated_at = SystemTime::now();

    storage
        .put_user(user)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    info!(username = %claims.sub, "User uploaded profile picture");

    Ok(StatusCode::CREATED)
}

/// GET /me/profile/avatar
pub async fn get_avatar(
    State(state): State<ApiState>,
    req: axum::extract::Request,
) -> Result<impl IntoResponse, ApiError> {
    let claims = extract_claims(&req, &state)?;
    let storage = require_storage!(state);
    let pictures_dir = require_profile_pictures_dir(&state)?;

    let user = load_claims_user(storage, &claims, &state).await?;

    let filename = user
        .profile_picture
        .ok_or_else(|| ApiError::NotFound("No profile picture set".to_string()))?;

    let file_path = pictures_dir.join(&filename);
    let data = tokio::fs::read(&file_path)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to read avatar file: {}", e)))?;

    let content_type = if filename.ends_with(".png") {
        "image/png"
    } else if filename.ends_with(".webp") {
        "image/webp"
    } else {
        "image/jpeg"
    };

    Ok((
        StatusCode::OK,
        [
            (axum::http::header::CONTENT_TYPE, content_type),
            (axum::http::header::CACHE_CONTROL, "private, max-age=3600"),
        ],
        data,
    ))
}

/// DELETE /me/profile/avatar
pub async fn delete_avatar(
    State(state): State<ApiState>,
    req: axum::extract::Request,
) -> Result<impl IntoResponse, ApiError> {
    let claims = extract_claims(&req, &state)?;
    let storage = require_storage!(state);
    let pictures_dir = require_profile_pictures_dir(&state)?;

    let mut user = load_claims_user(storage, &claims, &state).await?;

    if let Some(ref filename) = user.profile_picture {
        let file_path = pictures_dir.join(filename);
        let _ = tokio::fs::remove_file(&file_path).await;
    }

    user.profile_picture = None;
    user.updated_at = SystemTime::now();

    storage
        .put_user(user)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    info!(username = %claims.sub, "User deleted profile picture");

    Ok(StatusCode::NO_CONTENT)
}

// ---- Conference Self-Service Endpoints ----

/// GET /me/conferences -- List conferences owned by the current user
pub async fn list_my_conferences(
    State(state): State<ApiState>,
    req: axum::extract::Request,
) -> Result<impl IntoResponse, ApiError> {
    let claims = extract_claims(&req, &state)?;
    let storage = require_storage!(state);
    let tenant_id = resolve_tenant_from_claims(&claims, &state);

    let meetings = storage
        .list_meetings_by_owner(&tenant_id, &claims.sub)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let responses: Vec<super::conferences::MeetingResponse> = meetings
        .into_iter()
        .map(super::conferences::MeetingResponse::from)
        .collect();

    Ok(Json(responses))
}

/// GET /me/conferences/:id -- Get a specific conference owned by the current user
pub async fn get_my_conference(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    req: axum::extract::Request,
) -> Result<impl IntoResponse, ApiError> {
    let claims = extract_claims(&req, &state)?;
    let storage = require_storage!(state);
    let tenant_id = resolve_tenant_from_claims(&claims, &state);

    let meeting_id = fcp_core::MeetingId::from_str(&id)
        .map_err(|e| ApiError::BadRequest(format!("Invalid meeting ID: {}", e)))?;

    let meeting = storage
        .get_meeting(&meeting_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::NotFound("Conference not found".to_string()))?;

    // Verify ownership
    if meeting.tenant_id != tenant_id || meeting.owner_id.as_deref() != Some(&claims.sub) {
        return Err(ApiError::NotFound("Conference not found".to_string()));
    }

    Ok(Json(super::conferences::MeetingResponse::from(meeting)))
}

/// PATCH /me/conferences/:id -- Update a conference owned by the current user
pub async fn update_my_conference(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<UpdateMyConferenceRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let claims = extract_claims_from_headers(&headers, &state)?;
    let storage = require_storage!(state);
    let tenant_id = resolve_tenant_from_claims(&claims, &state);

    let meeting_id = fcp_core::MeetingId::from_str(&id)
        .map_err(|e| ApiError::BadRequest(format!("Invalid meeting ID: {}", e)))?;

    let mut meeting = storage
        .get_meeting(&meeting_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::NotFound("Conference not found".to_string()))?;

    // Verify ownership
    if meeting.tenant_id != tenant_id || meeting.owner_id.as_deref() != Some(&claims.sub) {
        return Err(ApiError::NotFound("Conference not found".to_string()));
    }

    // Apply allowed updates
    if let Some(name) = req.name {
        meeting.name = name;
    }
    if let Some(v) = req.guest_pin {
        meeting.guest_pin = v;
    }
    if let Some(v) = req.host_pin {
        meeting.host_pin = v;
    }
    if let Some(v) = req.require_guest_pin {
        meeting.require_guest_pin = v;
    }
    if let Some(v) = req.max_participants {
        meeting.max_participants = v;
    }
    if let Some(v) = req.wait_for_moderator {
        meeting.wait_for_moderator = v;
    }
    if let Some(v) = req.auto_record {
        meeting.auto_record = v;
    }
    if let Some(v) = req.default_locked {
        meeting.default_locked = v;
    }
    if let Some(v) = req.welcome_tts {
        meeting.welcome_tts = v;
    }
    meeting.updated_at = SystemTime::now();

    storage
        .put_meeting(meeting.clone())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(super::conferences::MeetingResponse::from(meeting)))
}

#[derive(Debug, Deserialize)]
pub struct UpdateMyConferenceRequest {
    pub name: Option<String>,
    pub guest_pin: Option<Option<String>>,
    pub host_pin: Option<Option<String>>,
    pub require_guest_pin: Option<bool>,
    pub max_participants: Option<usize>,
    pub wait_for_moderator: Option<bool>,
    pub auto_record: Option<bool>,
    pub default_locked: Option<bool>,
    pub welcome_tts: Option<Option<String>>,
}

/// Resolve the tenant ID from JWT claims, falling back to the default tenant.
fn resolve_tenant_from_claims(
    claims: &crate::v1::auth_session::Claims,
    state: &ApiState,
) -> fcp_core::TenantId {
    if !claims.tenant_id.is_empty() {
        if let Ok(tid) = claims.tenant_id.parse::<fcp_core::TenantId>() {
            return tid;
        }
    }
    state.default_tenant_id.clone()
}

/// Get the greetings directory or return an error.
fn require_greetings_dir(state: &ApiState) -> Result<&PathBuf, ApiError> {
    state.voicemail_greetings_dir.as_ref().ok_or_else(|| {
        ApiError::Internal("Voicemail greetings directory is not configured".to_string())
    })
}

/// Convert SystemTime to epoch seconds (f64).
fn system_time_to_epoch_secs(time: SystemTime) -> f64 {
    time.duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

/// Convert a VoicemailMessage into a MessageResponse.
fn message_to_response(msg: fcp_core::VoicemailMessage) -> MessageResponse {
    MessageResponse {
        id: msg.id,
        mailbox_id: msg.mailbox_id,
        caller: msg.caller,
        caller_name: msg.caller_name,
        status: msg.status.to_string(),
        priority: match msg.priority {
            fcp_core::MessagePriority::Normal => "normal".to_string(),
            fcp_core::MessagePriority::Urgent => "urgent".to_string(),
        },
        duration: msg.duration,
        recording_path: msg.recording_path.to_string_lossy().to_string(),
        transcription: msg.transcription,
        created_at: system_time_to_epoch_secs(msg.created_at),
        accessed_at: msg.accessed_at.map(system_time_to_epoch_secs),
    }
}
