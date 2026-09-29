//! Conference/Meeting management API endpoints.

use crate::{auth::TenantContext, error::ApiError, state::ApiState};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use fcp_core::meeting::{
    default_video_codecs, default_video_content_layout, default_video_content_share,
    default_video_fps, default_video_layout, default_video_max_participants,
    default_video_max_resolution, default_video_passthrough,
};
use fcp_core::storage::ConferenceStore;
use fcp_core::{Meeting, MeetingId, VideoCodec};
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

/// Meeting response.
#[derive(Debug, Serialize)]
pub struct MeetingResponse {
    pub id: String,
    pub tenant_id: String,
    pub room_number: String,
    pub name: String,
    pub guest_pin: Option<String>,
    pub host_pin: Option<String>,
    pub require_guest_pin: bool,
    pub max_participants: usize,
    pub wait_for_moderator: bool,
    pub enabled: bool,
    pub auto_record: bool,
    pub default_locked: bool,
    pub permanent: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pin_prompt_sound: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invalid_pin_sound: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub moh_sound: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pin_prompt_tts: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invalid_pin_tts: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub welcome_tts: Option<String>,
    pub play_entry_tone: bool,
    pub play_exit_tone: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub join_sound: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_sound: Option<String>,
    pub video_enabled: bool,
    pub video_layout: String,
    pub video_max_participants: u32,
    pub video_max_resolution: String,
    pub video_fps: u32,
    pub video_exclude_self: bool,
    pub video_codecs: Vec<VideoCodec>,
    /// When a participant whose view is a single source they can decode
    /// is sent that source's own stream: `auto`, `on` or `off` (§5.6).
    pub video_passthrough: String,
    /// Who may share a screen: `off`, `hosts` or `anyone` (§15.6).
    pub video_content_share: String,
    /// How the composite shows a shared screen: `presentation` or
    /// `content_only`.
    pub video_content_layout: String,
    #[serde(with = "crate::time::rfc3339")]
    pub created_at: SystemTime,
    #[serde(with = "crate::time::rfc3339")]
    pub updated_at: SystemTime,
}

impl From<Meeting> for MeetingResponse {
    fn from(m: Meeting) -> Self {
        MeetingResponse {
            id: m.id.to_string(),
            tenant_id: m.tenant_id.to_string(),
            room_number: m.room_number,
            name: m.name,
            guest_pin: m.guest_pin,
            host_pin: m.host_pin,
            require_guest_pin: m.require_guest_pin,
            max_participants: m.max_participants,
            wait_for_moderator: m.wait_for_moderator,
            enabled: m.enabled,
            auto_record: m.auto_record,
            default_locked: m.default_locked,
            permanent: m.permanent,
            owner_id: m.owner_id,
            pin_prompt_sound: m.pin_prompt_sound,
            invalid_pin_sound: m.invalid_pin_sound,
            moh_sound: m.moh_sound,
            pin_prompt_tts: m.pin_prompt_tts,
            invalid_pin_tts: m.invalid_pin_tts,
            welcome_tts: m.welcome_tts,
            play_entry_tone: m.play_entry_tone,
            play_exit_tone: m.play_exit_tone,
            join_sound: m.join_sound,
            exit_sound: m.exit_sound,
            video_enabled: m.video_enabled,
            video_layout: m.video_layout,
            video_max_participants: m.video_max_participants,
            video_max_resolution: m.video_max_resolution,
            video_fps: m.video_fps,
            video_exclude_self: m.video_exclude_self,
            video_codecs: m.video_codecs,
            video_passthrough: m.video_passthrough,
            video_content_share: m.video_content_share,
            video_content_layout: m.video_content_layout,
            created_at: m.created_at,
            updated_at: m.updated_at,
        }
    }
}

/// Create meeting request body.
#[derive(Debug, Deserialize)]
pub struct CreateMeetingRequest {
    pub room_number: String,
    pub name: String,
    #[serde(default)]
    pub guest_pin: Option<String>,
    #[serde(default)]
    pub host_pin: Option<String>,
    #[serde(default)]
    pub require_guest_pin: bool,
    #[serde(default)]
    pub max_participants: usize,
    #[serde(default)]
    pub wait_for_moderator: bool,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub auto_record: bool,
    #[serde(default)]
    pub default_locked: bool,
    #[serde(default)]
    pub permanent: bool,
    #[serde(default)]
    pub owner_id: Option<String>,
    #[serde(default)]
    pub pin_prompt_sound: Option<String>,
    #[serde(default)]
    pub invalid_pin_sound: Option<String>,
    #[serde(default)]
    pub moh_sound: Option<String>,
    #[serde(default)]
    pub pin_prompt_tts: Option<String>,
    #[serde(default)]
    pub invalid_pin_tts: Option<String>,
    #[serde(default)]
    pub welcome_tts: Option<String>,
    #[serde(default = "default_true")]
    pub play_entry_tone: bool,
    #[serde(default = "default_true")]
    pub play_exit_tone: bool,
    #[serde(default)]
    pub join_sound: Option<String>,
    #[serde(default)]
    pub exit_sound: Option<String>,
    #[serde(default)]
    pub video_enabled: bool,
    #[serde(default = "default_video_layout")]
    pub video_layout: String,
    #[serde(default = "default_video_max_participants")]
    pub video_max_participants: u32,
    #[serde(default = "default_video_max_resolution")]
    pub video_max_resolution: String,
    #[serde(default = "default_video_fps")]
    pub video_fps: u32,
    #[serde(default)]
    pub video_exclude_self: bool,
    #[serde(default = "default_video_codecs")]
    pub video_codecs: Vec<VideoCodec>,
    #[serde(default = "default_video_passthrough")]
    pub video_passthrough: String,
    #[serde(default = "default_video_content_share")]
    pub video_content_share: String,
    #[serde(default = "default_video_content_layout")]
    pub video_content_layout: String,
}

fn default_true() -> bool {
    true
}

fn ensure_meeting_tenant(
    meeting: Meeting,
    tenant_id: &fcp_core::TenantId,
    id: &str,
) -> Result<Meeting, ApiError> {
    if meeting.tenant_id != *tenant_id {
        return Err(ApiError::NotFound(format!("Meeting '{}' not found", id)));
    }
    Ok(meeting)
}

async fn room_number_exists_in_tenant(
    state: &ApiState,
    tenant_id: &fcp_core::TenantId,
    room_number: &str,
) -> Result<bool, ApiError> {
    let storage = require_storage!(state);
    storage
        .get_meeting_by_room_number(tenant_id, room_number)
        .await
        .map(|meeting| meeting.is_some())
        .map_err(|e| ApiError::Internal(e.to_string()))
}

/// Update meeting request body (all fields optional).
#[derive(Debug, Deserialize)]
pub struct UpdateMeetingRequest {
    pub name: Option<String>,
    pub guest_pin: Option<Option<String>>,
    pub host_pin: Option<Option<String>>,
    pub require_guest_pin: Option<bool>,
    pub max_participants: Option<usize>,
    pub wait_for_moderator: Option<bool>,
    pub enabled: Option<bool>,
    pub auto_record: Option<bool>,
    pub default_locked: Option<bool>,
    pub permanent: Option<bool>,
    pub owner_id: Option<Option<String>>,
    pub pin_prompt_sound: Option<Option<String>>,
    pub invalid_pin_sound: Option<Option<String>>,
    pub moh_sound: Option<Option<String>>,
    pub pin_prompt_tts: Option<Option<String>>,
    pub invalid_pin_tts: Option<Option<String>>,
    pub welcome_tts: Option<Option<String>>,
    pub play_entry_tone: Option<bool>,
    pub play_exit_tone: Option<bool>,
    pub join_sound: Option<Option<String>>,
    pub exit_sound: Option<Option<String>>,
    pub video_enabled: Option<bool>,
    pub video_layout: Option<String>,
    pub video_max_participants: Option<u32>,
    pub video_max_resolution: Option<String>,
    pub video_fps: Option<u32>,
    pub video_exclude_self: Option<bool>,
    pub video_codecs: Option<Vec<VideoCodec>>,
    pub video_passthrough: Option<String>,
    pub video_content_share: Option<String>,
    pub video_content_layout: Option<String>,
}

/// GET /conferences/meetings — List all meetings for the default tenant.
pub async fn list_meetings(
    State(state): State<ApiState>,
    tenant: TenantContext,
    axum::extract::Query(page): axum::extract::Query<crate::page::PageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);
    let tenant_id = &tenant.0;

    let meetings = storage
        .list_meetings(tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let responses: Vec<MeetingResponse> = meetings.into_iter().map(MeetingResponse::from).collect();
    Ok(Json(crate::page::paginate(
        responses,
        &page,
        crate::page::Order::Ascending,
        |r| format!("{:>20}/{}", r.room_number, r.id),
    )?))
}

/// POST /conferences/meetings — Create a new meeting.
pub async fn create_meeting(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Json(req): Json<CreateMeetingRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);
    let tenant_id = tenant.0;

    if req.room_number.is_empty() {
        return Err(ApiError::BadRequest("room_number is required".to_string()));
    }
    if req.name.is_empty() {
        return Err(ApiError::BadRequest("name is required".to_string()));
    }

    if room_number_exists_in_tenant(&state, &tenant_id, &req.room_number).await? {
        return Err(ApiError::Conflict(format!(
            "A meeting with room number '{}' already exists",
            req.room_number
        )));
    }

    let now = SystemTime::now();
    let meeting = Meeting {
        id: MeetingId::new(),
        tenant_id,
        room_number: req.room_number,
        name: req.name,
        guest_pin: req.guest_pin,
        host_pin: req.host_pin,
        require_guest_pin: req.require_guest_pin,
        max_participants: req.max_participants,
        wait_for_moderator: req.wait_for_moderator,
        enabled: req.enabled,
        auto_record: req.auto_record,
        default_locked: req.default_locked,
        permanent: req.permanent,
        owner_id: req.owner_id,
        pin_prompt_sound: req.pin_prompt_sound,
        invalid_pin_sound: req.invalid_pin_sound,
        moh_sound: req.moh_sound,
        pin_prompt_tts: req.pin_prompt_tts,
        invalid_pin_tts: req.invalid_pin_tts,
        welcome_tts: req.welcome_tts,
        play_entry_tone: req.play_entry_tone,
        play_exit_tone: req.play_exit_tone,
        join_sound: req.join_sound,
        exit_sound: req.exit_sound,
        video_enabled: req.video_enabled,
        video_layout: req.video_layout,
        video_max_participants: req.video_max_participants,
        video_max_resolution: req.video_max_resolution,
        video_fps: req.video_fps,
        video_exclude_self: req.video_exclude_self,
        video_codecs: req.video_codecs,
        video_passthrough: req.video_passthrough,
        video_content_share: req.video_content_share,
        video_content_layout: req.video_content_layout,
        created_at: now,
        updated_at: now,
    };
    meeting
        .validate_video_settings()
        .map_err(ApiError::BadRequest)?;

    storage
        .put_meeting(meeting.clone())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    info!(
        meeting_id = %meeting.id,
        room_number = %meeting.room_number,
        name = %meeting.name,
        "Meeting created via API"
    );

    Ok((StatusCode::CREATED, Json(MeetingResponse::from(meeting))))
}

/// GET /conferences/meetings/:id — Get a meeting by ID.
pub async fn get_meeting(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);

    let meeting_id = MeetingId::from_str(&id)
        .map_err(|e| ApiError::BadRequest(format!("Invalid meeting ID: {}", e)))?;

    let meeting = ensure_meeting_tenant(
        storage
            .get_meeting(&meeting_id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
            .ok_or_else(|| ApiError::NotFound(format!("Meeting '{}' not found", id)))?,
        &tenant.0,
        &id,
    )?;

    Ok(Json(MeetingResponse::from(meeting)))
}

/// PATCH /conferences/meetings/:id — Update a meeting.
pub async fn update_meeting(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(id): Path<String>,
    Json(req): Json<UpdateMeetingRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);

    let meeting_id = MeetingId::from_str(&id)
        .map_err(|e| ApiError::BadRequest(format!("Invalid meeting ID: {}", e)))?;

    let mut meeting = ensure_meeting_tenant(
        storage
            .get_meeting(&meeting_id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
            .ok_or_else(|| ApiError::NotFound(format!("Meeting '{}' not found", id)))?,
        &tenant.0,
        &id,
    )?;

    if let Some(name) = req.name {
        meeting.name = name;
    }
    if let Some(guest_pin) = req.guest_pin {
        meeting.guest_pin = guest_pin;
    }
    if let Some(host_pin) = req.host_pin {
        meeting.host_pin = host_pin;
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
    if let Some(v) = req.enabled {
        meeting.enabled = v;
    }
    if let Some(v) = req.auto_record {
        meeting.auto_record = v;
    }
    if let Some(v) = req.default_locked {
        meeting.default_locked = v;
    }
    if let Some(v) = req.permanent {
        meeting.permanent = v;
    }
    if let Some(v) = req.owner_id {
        meeting.owner_id = v;
    }
    if let Some(v) = req.pin_prompt_sound {
        meeting.pin_prompt_sound = v;
    }
    if let Some(v) = req.invalid_pin_sound {
        meeting.invalid_pin_sound = v;
    }
    if let Some(v) = req.moh_sound {
        meeting.moh_sound = v;
    }
    if let Some(v) = req.pin_prompt_tts {
        meeting.pin_prompt_tts = v;
    }
    if let Some(v) = req.invalid_pin_tts {
        meeting.invalid_pin_tts = v;
    }
    if let Some(v) = req.welcome_tts {
        meeting.welcome_tts = v;
    }
    if let Some(v) = req.play_entry_tone {
        meeting.play_entry_tone = v;
    }
    if let Some(v) = req.play_exit_tone {
        meeting.play_exit_tone = v;
    }
    if let Some(v) = req.join_sound {
        meeting.join_sound = v;
    }
    if let Some(v) = req.exit_sound {
        meeting.exit_sound = v;
    }
    if let Some(v) = req.video_enabled {
        meeting.video_enabled = v;
    }
    if let Some(v) = req.video_layout {
        meeting.video_layout = v;
    }
    if let Some(v) = req.video_max_participants {
        meeting.video_max_participants = v;
    }
    if let Some(v) = req.video_max_resolution {
        meeting.video_max_resolution = v;
    }
    if let Some(v) = req.video_fps {
        meeting.video_fps = v;
    }
    if let Some(v) = req.video_exclude_self {
        meeting.video_exclude_self = v;
    }
    if let Some(v) = req.video_passthrough {
        meeting.video_passthrough = v;
    }
    if let Some(v) = req.video_content_share {
        meeting.video_content_share = v;
    }
    if let Some(v) = req.video_content_layout {
        meeting.video_content_layout = v;
    }
    if let Some(v) = req.video_codecs {
        meeting.video_codecs = v;
    }
    meeting
        .validate_video_settings()
        .map_err(ApiError::BadRequest)?;
    meeting.updated_at = SystemTime::now();

    storage
        .put_meeting(meeting.clone())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    info!(
        meeting_id = %meeting.id,
        room_number = %meeting.room_number,
        "Meeting updated via API"
    );

    Ok(Json(MeetingResponse::from(meeting)))
}

/// DELETE /conferences/meetings/:id — Delete a meeting.
pub async fn delete_meeting(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);

    let meeting_id = MeetingId::from_str(&id)
        .map_err(|e| ApiError::BadRequest(format!("Invalid meeting ID: {}", e)))?;

    // Verify it exists
    let meeting = ensure_meeting_tenant(
        storage
            .get_meeting(&meeting_id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
            .ok_or_else(|| ApiError::NotFound(format!("Meeting '{}' not found", id)))?,
        &tenant.0,
        &id,
    )?;

    storage
        .delete_meeting(&meeting_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    info!(
        meeting_id = %meeting.id,
        room_number = %meeting.room_number,
        "Meeting deleted via API"
    );

    Ok(StatusCode::NO_CONTENT)
}

/// POST /conferences/meetings/:id/enable — Enable a meeting.
pub async fn enable_meeting(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);

    let meeting_id = MeetingId::from_str(&id)
        .map_err(|e| ApiError::BadRequest(format!("Invalid meeting ID: {}", e)))?;

    let mut meeting = ensure_meeting_tenant(
        storage
            .get_meeting(&meeting_id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
            .ok_or_else(|| ApiError::NotFound(format!("Meeting '{}' not found", id)))?,
        &tenant.0,
        &id,
    )?;

    meeting.enabled = true;
    meeting.updated_at = SystemTime::now();

    storage
        .put_meeting(meeting.clone())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    info!(meeting_id = %meeting.id, "Meeting enabled via API");

    Ok(Json(MeetingResponse::from(meeting)))
}

/// POST /conferences/meetings/:id/disable — Disable a meeting.
pub async fn disable_meeting(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);

    let meeting_id = MeetingId::from_str(&id)
        .map_err(|e| ApiError::BadRequest(format!("Invalid meeting ID: {}", e)))?;

    let mut meeting = ensure_meeting_tenant(
        storage
            .get_meeting(&meeting_id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
            .ok_or_else(|| ApiError::NotFound(format!("Meeting '{}' not found", id)))?,
        &tenant.0,
        &id,
    )?;

    meeting.enabled = false;
    meeting.updated_at = SystemTime::now();

    storage
        .put_meeting(meeting.clone())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    info!(meeting_id = %meeting.id, "Meeting disabled via API");

    Ok(Json(MeetingResponse::from(meeting)))
}

/// POST /conferences/meetings/:id/join-tokens — Mint a browser join token
/// for a meeting in the caller's tenant.
pub async fn mint_join_token(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(id): Path<String>,
    Json(req): Json<super::join_tokens::MintJoinTokenRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);
    let meeting_id = MeetingId::from_str(&id)
        .map_err(|e| ApiError::BadRequest(format!("Invalid meeting ID: {}", e)))?;
    let meeting = storage
        .get_meeting(&meeting_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::NotFound(format!("Meeting '{}' not found", id)))?;
    let meeting = ensure_meeting_tenant(meeting, &tenant.0, &id)?;
    let minted = super::join_tokens::mint_for_meeting(&state, &meeting, req)?;
    Ok((StatusCode::CREATED, Json(minted)))
}
