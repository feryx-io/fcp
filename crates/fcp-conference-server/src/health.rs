use axum::{
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    middleware,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
    routing::{delete, get, post},
    Json, Router,
};
use fcp_api_core::auth::{AuthConfig, validate_bearer_token};
use fcp_b2bua_core::CallManager;
use fcp_core::{DynStorage, Meeting, MeetingId, TenantId};
use fcp_events::{
    ConferenceEvent, EventFilter, EventPublisher, EventSubscriber, InMemoryEventBus, PlatformEvent,
};
use forge_conference::ConferenceBridge;
use futures::stream::Stream;
use serde::Deserialize;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
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
    pub rate_limit_rps: Option<u32>,
    pub event_subscriber: Arc<InMemoryEventBus>,
}

/// Public routes (no auth required).
fn public_router() -> Router<Arc<ConferenceState>> {
    Router::new()
        .route("/health", get(health_handler))
        .route("/metrics", get(metrics_handler))
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
        // Server-Sent Events stream
        .route("/events", get(sse_events_handler))
        // Recording management
        .route("/recordings", get(list_recordings_handler))
        .route(
            "/recordings/{filename}",
            get(download_recording_handler).delete(delete_recording_handler),
        )
}

/// Run the HTTP API server.
pub async fn run_http_server(
    bind_addr: SocketAddr,
    state: Arc<ConferenceState>,
) -> anyhow::Result<()> {
    let mut protected = if state.auth_enabled {
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

    if let Some(rps) = state.rate_limit_rps {
        if rps > 0 {
            info!(rps = rps, "HTTP API rate limiting enabled");
            let rate_limiter = Arc::new(RateLimiter::new(rps));
            protected = protected.layer(middleware::from_fn(move |req, next| {
                let limiter = Arc::clone(&rate_limiter);
                rate_limit_middleware(limiter, req, next)
            }));
        }
    }

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

async fn metrics_handler(State(state): State<Arc<ConferenceState>>) -> impl IntoResponse {
    let uptime = state.start_time.elapsed();
    let call_stats = state.call_manager.stats();
    let active_rooms = state.conference_handler.active_rooms();
    let total_participants: usize = active_rooms.iter().map(|(_, count)| count).sum();
    let active_sessions = state.conference_handler.session_count();

    let body = format!(
        "# HELP fcp_conference_active_rooms Number of active conference rooms\n\
         # TYPE fcp_conference_active_rooms gauge\n\
         fcp_conference_active_rooms {}\n\
         # HELP fcp_conference_active_participants Total participants across all rooms\n\
         # TYPE fcp_conference_active_participants gauge\n\
         fcp_conference_active_participants {}\n\
         # HELP fcp_conference_active_sessions Number of active SIP sessions\n\
         # TYPE fcp_conference_active_sessions gauge\n\
         fcp_conference_active_sessions {}\n\
         # HELP fcp_conference_calls_total Total calls since startup\n\
         # TYPE fcp_conference_calls_total counter\n\
         fcp_conference_calls_total {}\n\
         # HELP fcp_conference_uptime_seconds Server uptime in seconds\n\
         # TYPE fcp_conference_uptime_seconds gauge\n\
         fcp_conference_uptime_seconds {}\n",
        active_rooms.len(),
        total_participants,
        active_sessions,
        call_stats.total,
        uptime.as_secs(),
    );

    (StatusCode::OK, [("content-type", "text/plain; version=0.0.4; charset=utf-8")], body)
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
    /// Full path to audio file, OR just a filename to look up in announcements_dir.
    audio_path: String,
}

async fn announce_handler(
    State(state): State<Arc<ConferenceState>>,
    Path(room_number): Path<String>,
    Json(payload): Json<AnnounceRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    // If the path doesn't contain a separator, treat it as a filename in announcements_dir
    let audio_path = if payload.audio_path.contains('/') || payload.audio_path.contains('\\') {
        payload.audio_path.clone()
    } else {
        let conf = state.conference_handler.conference_config.read();
        format!("{}/{}", conf.announcements_dir, payload.audio_path)
    };

    match state.conference_handler.play_announcement(&room_number, &audio_path).await {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({ "status": "announced", "room": room_number, "audio_path": audio_path })),
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

async fn download_recording_handler(
    State(state): State<Arc<ConferenceState>>,
    Path(filename): Path<String>,
) -> impl IntoResponse {
    // Prevent path traversal
    if filename.contains('/') || filename.contains('\\') || filename.contains("..") {
        return (
            StatusCode::BAD_REQUEST,
            [("content-type", "application/json")],
            Body::from("{\"error\":\"Invalid filename\"}"),
        );
    }

    let recordings_dir = state.conference_handler.recordings_dir();
    let path = std::path::Path::new(&recordings_dir).join(&filename);

    if !path.exists() {
        return (
            StatusCode::NOT_FOUND,
            [("content-type", "application/json")],
            Body::from("{\"error\":\"Recording not found\"}"),
        );
    }

    let file = match tokio::fs::File::open(&path).await {
        Ok(f) => f,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [("content-type", "application/json")],
                Body::from(format!("{{\"error\":\"Failed to open file: {}\"}}", e)),
            );
        }
    };

    let stream = tokio_util::io::ReaderStream::new(file);
    (
        StatusCode::OK,
        [("content-type", "audio/wav")],
        Body::from_stream(stream),
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
// SSE Event Streaming
// =========================================================================

async fn sse_events_handler(
    State(state): State<Arc<ConferenceState>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let event_bus = Arc::clone(&state.event_subscriber);

    let stream = async_stream::stream! {
        let mut event_stream = match event_bus.subscribe(EventFilter::new()).await {
            Ok(s) => s,
            Err(e) => {
                warn!(error = %e, "Failed to subscribe for SSE stream");
                return;
            }
        };

        loop {
            match event_stream.next().await {
                Some(event) => {
                    let data = match serde_json::to_string(&event) {
                        Ok(json) => json,
                        Err(_) => continue,
                    };
                    yield Ok(Event::default().data(data));
                }
                None => {
                    // Stream ended, try to resubscribe
                    match event_bus.subscribe(EventFilter::new()).await {
                        Ok(s) => event_stream = s,
                        Err(_) => return,
                    }
                }
            }
        }
    };

    Sse::new(stream).keep_alive(KeepAlive::default())
}

// =========================================================================
// Rate Limiting
// =========================================================================

/// Simple token-bucket rate limiter using atomic counters.
struct RateLimiter {
    max_rps: u32,
    counter: AtomicU64,
    window_start: std::sync::Mutex<Instant>,
}

impl RateLimiter {
    fn new(max_rps: u32) -> Self {
        Self {
            max_rps,
            counter: AtomicU64::new(0),
            window_start: std::sync::Mutex::new(Instant::now()),
        }
    }

    fn check(&self) -> bool {
        let mut start = self.window_start.lock().unwrap();
        if start.elapsed() >= std::time::Duration::from_secs(1) {
            *start = Instant::now();
            self.counter.store(1, Ordering::Relaxed);
            true
        } else {
            let count = self.counter.fetch_add(1, Ordering::Relaxed);
            count < self.max_rps as u64
        }
    }
}

async fn rate_limit_middleware(
    limiter: Arc<RateLimiter>,
    req: axum::http::Request<Body>,
    next: axum::middleware::Next,
) -> axum::response::Response {
    if !limiter.check() {
        return (StatusCode::TOO_MANY_REQUESTS, "Rate limit exceeded").into_response();
    }
    next.run(req).await
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
