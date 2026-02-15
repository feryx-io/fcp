use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    middleware,
    routing::{delete, get, post},
    Json, Router,
};
use fcp_api_core::auth::{AuthConfig, validate_bearer_token};
use fcp_b2bua_core::CallManager;
use fcp_core::{DynStorage, Meeting, MeetingId, TenantId};
use fcp_events::{ConferenceEvent, EventPublisher, PlatformEvent};
use forge_conference::ConferenceBridge;
use serde::Deserialize;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Instant, SystemTime};
use tracing::{info, warn};

use crate::conference_handler::ConferenceCallHandler;

/// Shared state for the conference HTTP API.
pub struct ConferenceState {
    pub bridge: Arc<ConferenceBridge>,
    pub storage: Arc<DynStorage>,
    pub conference_handler: Arc<ConferenceCallHandler>,
    pub call_manager: Arc<CallManager>,
    pub event_bus: Arc<dyn EventPublisher>,
    pub start_time: Instant,
    pub auth_enabled: bool,
    pub auth_token: Option<String>,
}

/// Public routes (no auth required).
fn public_router() -> Router<Arc<ConferenceState>> {
    Router::new().route("/health", get(health_handler))
}

/// Protected routes (auth required when enabled).
fn protected_router() -> Router<Arc<ConferenceState>> {
    Router::new()
        .route("/status", get(status_handler))
        // Meeting CRUD (persistent config in DB)
        .route("/meetings", get(list_meetings_handler))
        .route("/meetings", post(create_meeting_handler))
        .route("/meetings/{id}", get(get_meeting_handler))
        .route("/meetings/{id}", post(update_meeting_handler))
        .route("/meetings/{id}", delete(delete_meeting_handler))
        // Runtime conference operations (active rooms on THIS instance)
        .route("/conferences", get(list_conferences_handler))
        .route("/conferences/{room_number}", get(get_conference_handler))
        .route(
            "/conferences/{room_number}/kick/{pid}",
            post(kick_participant_handler),
        )
        .route(
            "/conferences/{room_number}/mute/{pid}",
            post(mute_participant_handler),
        )
        .route(
            "/conferences/{room_number}/unmute/{pid}",
            post(unmute_participant_handler),
        )
        .route(
            "/conferences/{room_number}/lock",
            post(lock_room_handler),
        )
        .route(
            "/conferences/{room_number}/unlock",
            post(unlock_room_handler),
        )
        .route(
            "/conferences/{room_number}/recording/start",
            post(start_recording_handler),
        )
        .route(
            "/conferences/{room_number}/recording/stop",
            post(stop_recording_handler),
        )
        // Participant details and stats
        .route(
            "/conferences/{room_number}/participants/{pid}",
            get(get_participant_handler),
        )
        .route(
            "/conferences/{room_number}/participants/{pid}/gain",
            post(set_gain_handler),
        )
        .route(
            "/conferences/{room_number}/participants/{pid}/recording/start",
            post(start_participant_recording_handler),
        )
        .route(
            "/conferences/{room_number}/participants/{pid}/recording/stop",
            post(stop_participant_recording_handler),
        )
        .route(
            "/conferences/{room_number}/waiting",
            get(list_waiting_handler),
        )
        .route(
            "/conferences/{room_number}/promote/{pid}",
            post(promote_participant_handler),
        )
        .route(
            "/conferences/{room_number}/stats",
            get(room_stats_handler),
        )
        .route(
            "/conferences/{room_number}/announce",
            post(announce_handler),
        )
        // Admin: admit waiting participants
        .route(
            "/conferences/{room_number}/admit/{pid}",
            post(admit_participant_handler),
        )
        .route(
            "/conferences/{room_number}/admit-all",
            post(admit_all_handler),
        )
        // AI participant
        .route(
            "/conferences/{room_number}/ai",
            post(attach_ai_handler),
        )
        .route(
            "/conferences/{room_number}/ai",
            delete(detach_ai_handler),
        )
        .route(
            "/conferences/{room_number}/ai/status",
            get(ai_status_handler),
        )
        // Recording management
        .route("/recordings", get(list_recordings_handler))
        .route("/recordings/{filename}", delete(delete_recording_handler))
}

/// Run the HTTP API server.
pub async fn run_http_server(
    bind_addr: SocketAddr,
    state: Arc<ConferenceState>,
) -> anyhow::Result<()> {
    let protected = if state.auth_enabled {
        if let Some(ref token) = state.auth_token {
            let auth_config = AuthConfig::new(token.clone());
            info!("HTTP API authentication enabled");
            protected_router()
                .layer(middleware::from_fn(move |req, next| {
                    validate_bearer_token(auth_config.clone(), req, next)
                }))
        } else {
            warn!("auth_enabled=true but no auth_token configured, running without auth");
            protected_router()
        }
    } else {
        protected_router()
    };

    let app = public_router()
        .merge(protected)
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    info!(%bind_addr, "HTTP API server started");
    axum::serve(listener, app).await?;
    Ok(())
}

// =========================================================================
// Health & Status
// =========================================================================

async fn health_handler() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "healthy",
        "service": "fcp-conference"
    }))
}

async fn status_handler(State(state): State<Arc<ConferenceState>>) -> Json<serde_json::Value> {
    let uptime = state.start_time.elapsed();
    let call_stats = state.call_manager.stats();
    let active_rooms = state.conference_handler.active_rooms();

    let total_participants: usize = active_rooms.iter().map(|(_, count)| count).sum();

    Json(serde_json::json!({
        "status": "healthy",
        "service": "fcp-conference",
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_seconds": uptime.as_secs(),
        "active_calls": call_stats.active,
        "total_calls": call_stats.total,
        "active_rooms": active_rooms.len(),
        "total_participants": total_participants,
    }))
}

// =========================================================================
// Meeting CRUD
// =========================================================================

#[derive(Deserialize)]
struct CreateMeetingRequest {
    room_number: String,
    name: String,
    guest_pin: Option<String>,
    host_pin: Option<String>,
    #[serde(default)]
    require_guest_pin: bool,
    #[serde(default)]
    max_participants: usize,
    #[serde(default)]
    wait_for_moderator: bool,
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default)]
    auto_record: bool,
    #[serde(default)]
    default_locked: bool,
    #[serde(default)]
    permanent: bool,
}

fn default_true() -> bool {
    true
}

async fn list_meetings_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
) -> Json<serde_json::Value> {
    use fcp_core::storage::ConferenceStore;

    let tenant_id = extract_tenant_id(&headers);
    match state.storage.list_meetings(&tenant_id).await {
        Ok(meetings) => {
            let list: Vec<serde_json::Value> = meetings.iter().map(meeting_to_json).collect();
            Json(serde_json::json!({ "meetings": list }))
        }
        Err(e) => Json(serde_json::json!({
            "error": format!("Failed to list meetings: {}", e)
        })),
    }
}

async fn create_meeting_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Json(payload): Json<CreateMeetingRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    use fcp_core::storage::ConferenceStore;

    let tenant_id = extract_tenant_id(&headers);

    // Check for duplicate room number
    match state
        .storage
        .get_meeting_by_room_number(&tenant_id, &payload.room_number)
        .await
    {
        Ok(Some(_)) => {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({
                    "error": format!("Room number '{}' already exists", payload.room_number)
                })),
            );
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!("Storage error: {}", e)
                })),
            );
        }
        Ok(None) => {}
    }

    let now = SystemTime::now();
    let meeting = Meeting {
        id: MeetingId::new(),
        tenant_id,
        room_number: payload.room_number,
        name: payload.name,
        guest_pin: payload.guest_pin,
        host_pin: payload.host_pin,
        require_guest_pin: payload.require_guest_pin,
        max_participants: payload.max_participants,
        wait_for_moderator: payload.wait_for_moderator,
        enabled: payload.enabled,
        auto_record: payload.auto_record,
        default_locked: payload.default_locked,
        permanent: payload.permanent,
        created_at: now,
        updated_at: now,
    };

    match state.storage.put_meeting(meeting.clone()).await {
        Ok(()) => {
            publish_event(
                &state.event_bus,
                ConferenceEvent::MeetingCreated {
                    meeting_id: meeting.id.to_string(),
                    room_number: meeting.room_number.clone(),
                    name: meeting.name.clone(),
                    tenant_id: meeting.tenant_id,
                    timestamp: SystemTime::now(),
                },
            );
            (StatusCode::CREATED, Json(meeting_to_json(&meeting)))
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("Failed to create meeting: {}", e)
            })),
        ),
    }
}

async fn get_meeting_handler(
    State(state): State<Arc<ConferenceState>>,
    Path(id): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    use fcp_core::storage::ConferenceStore;

    let meeting_id: MeetingId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "Invalid meeting ID" })),
            );
        }
    };

    match state.storage.get_meeting(&meeting_id).await {
        Ok(Some(meeting)) => (StatusCode::OK, Json(meeting_to_json(&meeting))),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Meeting not found" })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("Failed to get meeting: {}", e)
            })),
        ),
    }
}

#[derive(Deserialize)]
struct UpdateMeetingRequest {
    name: Option<String>,
    guest_pin: Option<String>,
    host_pin: Option<String>,
    require_guest_pin: Option<bool>,
    max_participants: Option<usize>,
    wait_for_moderator: Option<bool>,
    enabled: Option<bool>,
    auto_record: Option<bool>,
    default_locked: Option<bool>,
    permanent: Option<bool>,
}

async fn update_meeting_handler(
    State(state): State<Arc<ConferenceState>>,
    Path(id): Path<String>,
    Json(payload): Json<UpdateMeetingRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    use fcp_core::storage::ConferenceStore;

    let meeting_id: MeetingId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "Invalid meeting ID" })),
            );
        }
    };

    let mut meeting = match state.storage.get_meeting(&meeting_id).await {
        Ok(Some(m)) => m,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "Meeting not found" })),
            );
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!("Failed to get meeting: {}", e)
                })),
            );
        }
    };

    // Apply updates, tracking which fields changed
    let mut changed_fields = Vec::new();
    if let Some(name) = payload.name {
        meeting.name = name;
        changed_fields.push("name".to_string());
    }
    if let Some(guest_pin) = payload.guest_pin {
        meeting.guest_pin = Some(guest_pin);
        changed_fields.push("guest_pin".to_string());
    }
    if let Some(host_pin) = payload.host_pin {
        meeting.host_pin = Some(host_pin);
        changed_fields.push("host_pin".to_string());
    }
    if let Some(v) = payload.require_guest_pin {
        meeting.require_guest_pin = v;
        changed_fields.push("require_guest_pin".to_string());
    }
    if let Some(v) = payload.max_participants {
        meeting.max_participants = v;
        changed_fields.push("max_participants".to_string());
    }
    if let Some(v) = payload.wait_for_moderator {
        meeting.wait_for_moderator = v;
        changed_fields.push("wait_for_moderator".to_string());
    }
    if let Some(v) = payload.enabled {
        meeting.enabled = v;
        changed_fields.push("enabled".to_string());
    }
    if let Some(v) = payload.auto_record {
        meeting.auto_record = v;
        changed_fields.push("auto_record".to_string());
    }
    if let Some(v) = payload.default_locked {
        meeting.default_locked = v;
        changed_fields.push("default_locked".to_string());
    }
    if let Some(v) = payload.permanent {
        meeting.permanent = v;
        changed_fields.push("permanent".to_string());
    }
    meeting.updated_at = SystemTime::now();

    match state.storage.put_meeting(meeting.clone()).await {
        Ok(()) => {
            // Invalidate meeting cache
            state
                .conference_handler
                .invalidate_meeting_cache(&meeting.tenant_id, &meeting.room_number);
            publish_event(
                &state.event_bus,
                ConferenceEvent::MeetingUpdated {
                    meeting_id: meeting.id.to_string(),
                    room_number: meeting.room_number.clone(),
                    name: meeting.name.clone(),
                    tenant_id: meeting.tenant_id,
                    changed_fields,
                    timestamp: SystemTime::now(),
                },
            );
            (StatusCode::OK, Json(meeting_to_json(&meeting)))
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("Failed to update meeting: {}", e)
            })),
        ),
    }
}

async fn delete_meeting_handler(
    State(state): State<Arc<ConferenceState>>,
    Path(id): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    use fcp_core::storage::ConferenceStore;

    let meeting_id: MeetingId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "Invalid meeting ID" })),
            );
        }
    };

    // Fetch meeting details before deletion for the event
    let meeting_info = state.storage.get_meeting(&meeting_id).await.ok().flatten();

    match state.storage.delete_meeting(&meeting_id).await {
        Ok(()) => {
            if let Some(ref meeting) = meeting_info {
                // Invalidate meeting cache
                state
                    .conference_handler
                    .invalidate_meeting_cache(&meeting.tenant_id, &meeting.room_number);
            }
            if let Some(meeting) = meeting_info {
                publish_event(
                    &state.event_bus,
                    ConferenceEvent::MeetingDeleted {
                        meeting_id: meeting.id.to_string(),
                        room_number: meeting.room_number,
                        tenant_id: meeting.tenant_id,
                        timestamp: SystemTime::now(),
                    },
                );
            }
            (
                StatusCode::OK,
                Json(serde_json::json!({ "status": "deleted" })),
            )
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("Failed to delete meeting: {}", e)
            })),
        ),
    }
}

// =========================================================================
// Runtime Conference Operations
// =========================================================================

async fn list_conferences_handler(
    State(state): State<Arc<ConferenceState>>,
) -> Json<serde_json::Value> {
    let rooms = state.conference_handler.active_rooms();
    let list: Vec<serde_json::Value> = rooms
        .iter()
        .map(|(room_id, count)| {
            serde_json::json!({
                "room_number": room_id,
                "participants": count,
            })
        })
        .collect();

    Json(serde_json::json!({ "conferences": list }))
}

async fn get_conference_handler(
    State(state): State<Arc<ConferenceState>>,
    Path(room_number): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    match state.bridge.get_room(&room_number) {
        Ok(room) => {
            let participants = room.participants();
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "room_number": room_number,
                    "participant_count": room.participant_count(),
                    "participants": participants,
                    "is_locked": room.is_locked(),
                    "is_recording": room.is_recording(),
                })),
            )
        }
        Err(_) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Conference room not active" })),
        ),
    }
}

async fn kick_participant_handler(
    State(state): State<Arc<ConferenceState>>,
    Path((room_number, pid)): Path<(String, String)>,
) -> (StatusCode, Json<serde_json::Value>) {
    // Verify room exists
    if state.bridge.get_room(&room_number).is_err() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Conference room not active" })),
        );
    }

    // Kick via conference handler: sends SIP BYE, aborts bridge, deallocates port
    match state.conference_handler.kick_participant(&pid).await {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({ "status": "kicked", "participant": pid })),
        ),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": format!("{}", e) })),
        ),
    }
}

async fn mute_participant_handler(
    State(state): State<Arc<ConferenceState>>,
    Path((room_number, pid)): Path<(String, String)>,
) -> (StatusCode, Json<serde_json::Value>) {
    match state.bridge.get_room(&room_number) {
        Ok(room) => {
            match room.set_participant_state(&pid, forge_mixer::ParticipantState::Muted) {
                Ok(()) => (
                    StatusCode::OK,
                    Json(serde_json::json!({ "status": "muted", "participant": pid })),
                ),
                Err(e) => (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({ "error": format!("{}", e) })),
                ),
            }
        }
        Err(_) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Conference room not active" })),
        ),
    }
}

async fn unmute_participant_handler(
    State(state): State<Arc<ConferenceState>>,
    Path((room_number, pid)): Path<(String, String)>,
) -> (StatusCode, Json<serde_json::Value>) {
    match state.bridge.get_room(&room_number) {
        Ok(room) => {
            match room.set_participant_state(&pid, forge_mixer::ParticipantState::Active) {
                Ok(()) => (
                    StatusCode::OK,
                    Json(serde_json::json!({ "status": "unmuted", "participant": pid })),
                ),
                Err(e) => (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({ "error": format!("{}", e) })),
                ),
            }
        }
        Err(_) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Conference room not active" })),
        ),
    }
}

async fn lock_room_handler(
    State(state): State<Arc<ConferenceState>>,
    Path(room_number): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    match state.bridge.get_room(&room_number) {
        Ok(room) => {
            room.lock();
            publish_event(
                &state.event_bus,
                ConferenceEvent::RoomLocked {
                    room_number: room_number.clone(),
                    tenant_id: TenantId::nil(),
                    timestamp: SystemTime::now(),
                },
            );
            (
                StatusCode::OK,
                Json(serde_json::json!({ "status": "locked", "room": room_number })),
            )
        }
        Err(_) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Conference room not active" })),
        ),
    }
}

async fn unlock_room_handler(
    State(state): State<Arc<ConferenceState>>,
    Path(room_number): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    match state.bridge.get_room(&room_number) {
        Ok(room) => {
            room.unlock();
            publish_event(
                &state.event_bus,
                ConferenceEvent::RoomUnlocked {
                    room_number: room_number.clone(),
                    tenant_id: TenantId::nil(),
                    timestamp: SystemTime::now(),
                },
            );
            (
                StatusCode::OK,
                Json(serde_json::json!({ "status": "unlocked", "room": room_number })),
            )
        }
        Err(_) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Conference room not active" })),
        ),
    }
}

async fn start_recording_handler(
    State(state): State<Arc<ConferenceState>>,
    Path(room_number): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    match state.bridge.get_room(&room_number) {
        Ok(room) => {
            let recording_path = format!(
                "{}/{}-{}.wav",
                state.conference_handler.recordings_dir(),
                room_number,
                chrono_timestamp()
            );
            match room.start_recording(&recording_path, None).await {
                Ok(()) => {
                    publish_event(
                        &state.event_bus,
                        ConferenceEvent::RecordingStarted {
                            room_number: room_number.clone(),
                            tenant_id: TenantId::nil(),
                            timestamp: SystemTime::now(),
                        },
                    );
                    (
                        StatusCode::OK,
                        Json(serde_json::json!({
                            "status": "recording_started",
                            "path": recording_path,
                        })),
                    )
                }
                Err(e) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": format!("{}", e) })),
                ),
            }
        }
        Err(_) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Conference room not active" })),
        ),
    }
}

async fn stop_recording_handler(
    State(state): State<Arc<ConferenceState>>,
    Path(room_number): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    match state.bridge.get_room(&room_number) {
        Ok(room) => match room.stop_recording() {
            Ok(()) => {
                publish_event(
                    &state.event_bus,
                    ConferenceEvent::RecordingStopped {
                        room_number: room_number.clone(),
                        tenant_id: TenantId::nil(),
                        timestamp: SystemTime::now(),
                    },
                );
                (
                    StatusCode::OK,
                    Json(serde_json::json!({ "status": "recording_stopped" })),
                )
            }
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("{}", e) })),
            ),
        },
        Err(_) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Conference room not active" })),
        ),
    }
}

// =========================================================================
// Participant Details & Stats
// =========================================================================

async fn get_participant_handler(
    State(state): State<Arc<ConferenceState>>,
    Path((room_number, pid)): Path<(String, String)>,
) -> (StatusCode, Json<serde_json::Value>) {
    match state.conference_handler.get_participant_details(&room_number, &pid) {
        Ok(details) => (StatusCode::OK, Json(details)),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": format!("{}", e) })),
        ),
    }
}

#[derive(Deserialize)]
struct SetGainRequest {
    gain: f32,
}

async fn set_gain_handler(
    State(state): State<Arc<ConferenceState>>,
    Path((room_number, pid)): Path<(String, String)>,
    Json(payload): Json<SetGainRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    match state.conference_handler.set_gain(&room_number, &pid, payload.gain) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({ "status": "gain_set", "participant": pid, "gain": payload.gain })),
        ),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": format!("{}", e) })),
        ),
    }
}

async fn list_waiting_handler(
    State(state): State<Arc<ConferenceState>>,
    Path(room_number): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    match state.conference_handler.list_waiting(&room_number) {
        Ok(waiting) => (
            StatusCode::OK,
            Json(serde_json::json!({ "room_number": room_number, "waiting": waiting })),
        ),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": format!("{}", e) })),
        ),
    }
}

async fn promote_participant_handler(
    State(state): State<Arc<ConferenceState>>,
    Path((room_number, pid)): Path<(String, String)>,
) -> (StatusCode, Json<serde_json::Value>) {
    match state.conference_handler.promote_participant(&room_number, &pid) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({ "status": "promoted", "participant": pid })),
        ),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": format!("{}", e) })),
        ),
    }
}

async fn room_stats_handler(
    State(state): State<Arc<ConferenceState>>,
    Path(room_number): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    match state.conference_handler.get_room_stats(&room_number) {
        Ok(stats) => (StatusCode::OK, Json(stats)),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": format!("{}", e) })),
        ),
    }
}

async fn start_participant_recording_handler(
    State(state): State<Arc<ConferenceState>>,
    Path((room_number, pid)): Path<(String, String)>,
) -> (StatusCode, Json<serde_json::Value>) {
    match state.conference_handler.start_participant_recording(&room_number, &pid).await {
        Ok(path) => (
            StatusCode::OK,
            Json(serde_json::json!({ "status": "recording_started", "participant": pid, "path": path })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("{}", e) })),
        ),
    }
}

async fn stop_participant_recording_handler(
    State(state): State<Arc<ConferenceState>>,
    Path((room_number, pid)): Path<(String, String)>,
) -> (StatusCode, Json<serde_json::Value>) {
    match state.conference_handler.stop_participant_recording(&room_number, &pid) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({ "status": "recording_stopped", "participant": pid })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("{}", e) })),
        ),
    }
}

#[derive(Deserialize)]
struct AnnounceRequest {
    audio_path: String,
}

async fn announce_handler(
    State(state): State<Arc<ConferenceState>>,
    Path(room_number): Path<String>,
    Json(payload): Json<AnnounceRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    match state.conference_handler.play_announcement(&room_number, &payload.audio_path).await {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({ "status": "announced", "room": room_number })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("{}", e) })),
        ),
    }
}

// =========================================================================
// Admin: Admit Waiting Participants
// =========================================================================

async fn admit_participant_handler(
    State(state): State<Arc<ConferenceState>>,
    Path((room_number, pid)): Path<(String, String)>,
) -> (StatusCode, Json<serde_json::Value>) {
    match state.conference_handler.admit_waiting(&room_number, &pid) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({ "status": "admitted", "participant": pid })),
        ),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": format!("{}", e) })),
        ),
    }
}

async fn admit_all_handler(
    State(state): State<Arc<ConferenceState>>,
    Path(room_number): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    match state.conference_handler.admit_all_waiting(&room_number) {
        Ok(count) => (
            StatusCode::OK,
            Json(serde_json::json!({ "status": "admitted_all", "count": count })),
        ),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": format!("{}", e) })),
        ),
    }
}

// =========================================================================
// AI Participant
// =========================================================================

async fn attach_ai_handler(
    State(state): State<Arc<ConferenceState>>,
    Path(room_number): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    match state.conference_handler.attach_ai(&room_number).await {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({ "status": "ai_attached", "room": room_number })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("{}", e) })),
        ),
    }
}

async fn detach_ai_handler(
    State(state): State<Arc<ConferenceState>>,
    Path(room_number): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    match state.conference_handler.detach_ai(&room_number).await {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({ "status": "ai_detached", "room": room_number })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("{}", e) })),
        ),
    }
}

async fn ai_status_handler(
    State(state): State<Arc<ConferenceState>>,
    Path(room_number): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    match state.conference_handler.ai_status(&room_number) {
        Ok(status) => (StatusCode::OK, Json(status)),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": format!("{}", e) })),
        ),
    }
}

// =========================================================================
// Recording Management
// =========================================================================

async fn list_recordings_handler(
    State(state): State<Arc<ConferenceState>>,
) -> (StatusCode, Json<serde_json::Value>) {
    let recordings_dir = state.conference_handler.recordings_dir();

    let entries = match std::fs::read_dir(recordings_dir) {
        Ok(entries) => entries,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Cannot read recordings dir: {}", e) })),
            );
        }
    };

    let mut recordings = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("wav") {
            let filename = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let meta = entry.metadata().ok();
            let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
            let modified = meta
                .as_ref()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            recordings.push(serde_json::json!({
                "filename": filename,
                "size_bytes": size,
                "modified_at": modified,
            }));
        }
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({ "recordings": recordings, "count": recordings.len() })),
    )
}

async fn delete_recording_handler(
    State(state): State<Arc<ConferenceState>>,
    Path(filename): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    // Prevent path traversal
    if filename.contains('/') || filename.contains('\\') || filename.contains("..") {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Invalid filename" })),
        );
    }

    let recordings_dir = state.conference_handler.recordings_dir();
    let path = std::path::Path::new(&recordings_dir).join(&filename);

    if !path.exists() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Recording not found" })),
        );
    }

    match std::fs::remove_file(&path) {
        Ok(()) => {
            info!(filename = %filename, "Recording deleted");
            (
                StatusCode::OK,
                Json(serde_json::json!({ "status": "deleted", "filename": filename })),
            )
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Failed to delete: {}", e) })),
        ),
    }
}

// =========================================================================
// Helpers
// =========================================================================

/// Extract tenant ID from request headers.
/// Checks X-Tenant-ID header first, falls back to TenantId::nil().
fn extract_tenant_id(headers: &HeaderMap) -> TenantId {
    if let Some(value) = headers.get("X-Tenant-ID") {
        if let Ok(s) = value.to_str() {
            if let Ok(tid) = s.parse::<TenantId>() {
                return tid;
            }
        }
    }
    TenantId::nil()
}

fn meeting_to_json(meeting: &Meeting) -> serde_json::Value {
    serde_json::json!({
        "id": meeting.id.to_string(),
        "tenant_id": meeting.tenant_id.to_string(),
        "room_number": meeting.room_number,
        "name": meeting.name,
        "guest_pin": meeting.guest_pin,
        "host_pin": meeting.host_pin,
        "require_guest_pin": meeting.require_guest_pin,
        "max_participants": meeting.max_participants,
        "wait_for_moderator": meeting.wait_for_moderator,
        "enabled": meeting.enabled,
        "auto_record": meeting.auto_record,
        "default_locked": meeting.default_locked,
        "permanent": meeting.permanent,
        "created_at": meeting.created_at
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        "updated_at": meeting.updated_at
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    })
}

fn publish_event(event_bus: &Arc<dyn EventPublisher>, event: ConferenceEvent) {
    let event_bus = Arc::clone(event_bus);
    let platform_event = PlatformEvent::Conference(event);
    tokio::spawn(async move {
        if let Err(e) = event_bus.publish(platform_event).await {
            warn!(error = %e, "Failed to publish conference event");
        }
    });
}

fn chrono_timestamp() -> String {
    let secs = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("{}", secs)
}
