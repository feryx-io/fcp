use axum::{
    body::Body,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, StatusCode},
    middleware,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
    routing::{delete, get, post, put},
    Json, Router,
};
use fcp_api_core::auth::{validate_bearer_token, AuthConfig, PLATFORM_TENANT_HEADER};
use fcp_b2bua_core::CallManager;
use fcp_core::storage::{ConferenceStore, TenantStore};
use fcp_core::{DynStorage, Meeting, MeetingId, TenantId};
use fcp_events::{
    ConferenceEvent, EventFilter, EventPublisher, EventSubscriber, InMemoryEventBus, PlatformEvent,
};
use forge_conference::ConferenceRoom;
use futures::stream::Stream;
use serde::Deserialize;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Instant, SystemTime};
use tracing::{info, warn};

use crate::conference_handler::ConferenceCallHandler;
use crate::control::{ClusterControl, PeerOutcome, NODE_HEADER};
use crate::rooms::RoomKey;

/// Shared state for the conference HTTP API.
pub struct ConferenceState {
    pub bridge: Arc<crate::rooms::RoomBridge>,
    pub storage: Arc<DynStorage>,
    /// The read replica as of its last probe (`docs/DEPLOYMENT_GAPS.md` §1.7).
    pub replica: fcp_storage_postgres::ReplicaHealth,
    pub conference_handler: Arc<ConferenceCallHandler>,
    pub call_manager: Arc<CallManager>,
    pub event_bus: Arc<dyn EventPublisher>,
    pub default_tenant_id: TenantId,
    pub start_time: Instant,
    pub auth_enabled: bool,
    pub auth_token: Option<String>,
    pub rate_limit_rps: Option<u32>,
    pub event_subscriber: Arc<InMemoryEventBus>,
    /// Counter for events dropped by the circuit breaker (exposed via /metrics)
    pub events_dropped: Arc<AtomicU64>,
    /// Maximum HTTP request body size in bytes (from config).
    pub max_body_size_bytes: usize,
    /// Cross-node control: forwards room-wide and participant operations
    /// to the other nodes hosting a room. `None` outside a cluster or
    /// without `cluster.secret`.
    pub control: Option<Arc<ClusterControl>>,
    /// Secret peer nodes must present on `/internal` routes; `None`
    /// disables those routes.
    pub cluster_secret: Option<String>,
    /// Leaving service (`docs/CAPACITY_AND_SURVIVABILITY.md` §6).
    pub drain: Arc<fcp_sip::drain::Drain>,
    /// Cluster membership, when the node is clustered: what `/status` and
    /// `/metrics` report of its view (`docs/MULTI_REGION_FOLLOWUPS.md` §2.3).
    #[cfg(feature = "storage-redis")]
    pub cluster: Option<Arc<fcp_cluster::ClusterManager>>,
}

impl ConferenceState {
    /// Which release every node runs, when this one is clustered.
    fn cluster_versions(&self) -> Option<fcp_cluster::ClusterVersions> {
        #[cfg(feature = "storage-redis")]
        {
            self.cluster.as_ref().map(|c| c.versions())
        }
        #[cfg(not(feature = "storage-redis"))]
        {
            None
        }
    }

    /// This node's view of the cluster, when it is clustered.
    fn cluster_view(&self) -> Option<fcp_cluster::ClusterView> {
        #[cfg(feature = "storage-redis")]
        {
            self.cluster.as_ref().map(|c| c.view())
        }
        #[cfg(not(feature = "storage-redis"))]
        {
            None
        }
    }
}

/// Public routes (no auth required).
fn public_router() -> Router<Arc<ConferenceState>> {
    Router::new()
        .route("/health", get(health_handler))
        // What a load balancer reads to stop choosing a draining node; it
        // has no credential to present (§6).
        .route("/ready", get(ready_handler))
}

/// Protected routes (auth required when enabled).
fn protected_router() -> Router<Arc<ConferenceState>> {
    Router::new()
        .route("/status", get(status_handler))
        .route("/metrics", get(metrics_handler))
        // Leaving service: start a drain, or call one off (§6).
        .route(
            "/drain",
            post(start_drain_handler).delete(stop_drain_handler),
        )
        // Meeting CRUD (persistent config in DB)
        .route("/meetings", get(list_meetings_handler))
        .route("/meetings", post(create_meeting_handler))
        .route("/meetings/:id", get(get_meeting_handler))
        .route("/meetings/:id", post(update_meeting_handler))
        .route("/meetings/:id", delete(delete_meeting_handler))
        // Runtime conference operations (active rooms on THIS instance)
        .route("/conferences", get(list_conferences_handler))
        .route(
            "/conferences/:room_number",
            get(get_conference_handler).delete(destroy_conference_handler),
        )
        .route(
            "/conferences/:room_number/kick/:pid",
            post(kick_participant_handler),
        )
        .route(
            "/conferences/:room_number/mute/:pid",
            post(mute_participant_handler),
        )
        .route(
            "/conferences/:room_number/unmute/:pid",
            post(unmute_participant_handler),
        )
        .route("/conferences/:room_number/lock", post(lock_room_handler))
        .route(
            "/conferences/:room_number/unlock",
            post(unlock_room_handler),
        )
        .route(
            "/conferences/:room_number/recording/start",
            post(start_recording_handler),
        )
        .route(
            "/conferences/:room_number/recording/stop",
            post(stop_recording_handler),
        )
        // Participant details and stats
        .route(
            "/conferences/:room_number/participants/:pid",
            get(get_participant_handler),
        )
        .route(
            "/conferences/:room_number/participants/:pid/gain",
            post(set_gain_handler),
        )
        .route(
            "/conferences/:room_number/participants/:pid/recording/start",
            post(start_participant_recording_handler),
        )
        .route(
            "/conferences/:room_number/participants/:pid/recording/stop",
            post(stop_participant_recording_handler),
        )
        .route(
            "/conferences/:room_number/waiting",
            get(list_waiting_handler),
        )
        .route(
            "/conferences/:room_number/promote/:pid",
            post(promote_participant_handler),
        )
        // The audience's voice (`docs/WEBINARS.md` §2.5): a participant's
        // own hand by their call, a host lowering hands, and the stage.
        .route("/conferences/:room_number/hand", post(own_hand_handler))
        .route(
            "/conferences/:room_number/participants/:pid/hand",
            delete(lower_hand_handler),
        )
        .route(
            "/conferences/:room_number/hands",
            delete(lower_hands_handler),
        )
        .route(
            "/conferences/:room_number/participants/:pid/stage",
            post(stage_on_handler).delete(stage_off_handler),
        )
        // A webinar's life (`docs/WEBINARS.md` §2.6, W4): its sitting and
        // phase, taken live and ended by a host through the API.
        .route(
            "/conferences/:room_number/webinar",
            get(webinar_status_handler),
        )
        .route(
            "/conferences/:room_number/webinar/live",
            post(webinar_live_handler),
        )
        .route(
            "/conferences/:room_number/webinar/end",
            post(webinar_end_handler),
        )
        .route("/conferences/:room_number/audience", get(audience_handler))
        // Questions (`docs/WEBINARS.md` §2.5, W3.2): asked and upvoted by
        // a participant's call, moderated by a host through the API.
        .route(
            "/conferences/:room_number/questions",
            get(list_questions_handler).post(ask_question_handler),
        )
        .route(
            "/conferences/:room_number/questions/:qid/upvote",
            post(upvote_question_handler),
        )
        .route(
            "/conferences/:room_number/questions/:qid/publish",
            post(publish_question_handler),
        )
        .route(
            "/conferences/:room_number/questions/:qid/answer",
            post(answer_question_handler),
        )
        .route(
            "/conferences/:room_number/questions/:qid/dismiss",
            post(dismiss_question_handler),
        )
        // Chat (`docs/WEBINARS.md` §2.5, W3b): posted by a participant's
        // call, deleted and muted by a host or panelist.
        .route(
            "/conferences/:room_number/chat",
            get(list_chat_handler).post(post_chat_handler),
        )
        .route(
            "/conferences/:room_number/chat/:mid/delete",
            post(delete_chat_handler),
        )
        .route(
            "/conferences/:room_number/participants/:pid/chat-mute",
            post(mute_chat_handler),
        )
        .route("/conferences/:room_number/stats", get(room_stats_handler))
        .route("/conferences/:room_number/announce", post(announce_handler))
        // Video (design §12): status, layout, pin, spotlight, per-participant on/off
        .route("/conferences/:room_number/video", get(video_status_handler))
        .route(
            "/conferences/:room_number/video/layout",
            put(video_layout_handler),
        )
        .route(
            "/conferences/:room_number/video/pin/:pid",
            post(video_pin_handler),
        )
        .route(
            "/conferences/:room_number/video/pin",
            delete(video_unpin_handler),
        )
        .route(
            "/conferences/:room_number/video/spotlight/:pid",
            post(video_spotlight_handler),
        )
        .route(
            "/conferences/:room_number/video/spotlight",
            delete(video_unspotlight_handler),
        )
        .route(
            "/conferences/:room_number/participants/:pid/video/:state",
            post(participant_video_handler),
        )
        // A shared screen (§15.6): a host ends the share, and how the
        // composite shows one.
        .route(
            "/conferences/:room_number/video/content/stop",
            post(content_stop_handler),
        )
        .route(
            "/conferences/:room_number/video/content-layout",
            put(content_layout_handler),
        )
        // Admin: admit waiting participants
        .route(
            "/conferences/:room_number/admit/:pid",
            post(admit_participant_handler),
        )
        .route(
            "/conferences/:room_number/admit-all",
            post(admit_all_handler),
        )
        // Dial-out: invite an external participant by calling them
        .route("/conferences/:room_number/dial-out", post(dial_out_handler))
        // AI participant
        .route("/conferences/:room_number/ai", post(attach_ai_handler))
        .route("/conferences/:room_number/ai", delete(detach_ai_handler))
        .route(
            "/conferences/:room_number/ai/status",
            get(ai_status_handler),
        )
        // Server-Sent Events stream
        .route("/events", get(sse_events_handler))
        // Recording management
        .route("/recordings", get(list_recordings_handler))
        .route(
            "/recordings/:filename",
            get(download_recording_handler).delete(delete_recording_handler),
        )
        // A webinar's HLS stream (`docs/WEBINARS.md` §2.8, W6.2).
        .route("/hls/:sitting/:file", get(hls_file_handler))
}

/// Routes peer conference nodes call to apply a control operation on
/// this node only (cross-node control, phase 3). Authenticated with the
/// shared `cluster.secret`, never forwarded further.
fn internal_router(state: Arc<ConferenceState>) -> Router<Arc<ConferenceState>> {
    Router::new()
        .route(
            "/internal/conferences/:room_number",
            delete(internal_destroy_handler),
        )
        .route(
            "/internal/conferences/:room_number/lock",
            post(internal_lock_handler),
        )
        .route(
            "/internal/conferences/:room_number/unlock",
            post(internal_unlock_handler),
        )
        .route(
            "/internal/conferences/:room_number/kick/:pid",
            post(internal_kick_handler),
        )
        .route(
            "/internal/conferences/:room_number/mute/:pid",
            post(internal_mute_handler),
        )
        .route(
            "/internal/conferences/:room_number/unmute/:pid",
            post(internal_unmute_handler),
        )
        .route(
            "/internal/conferences/:room_number/participants/:pid/gain",
            post(internal_gain_handler),
        )
        .route(
            "/internal/conferences/:room_number/announce",
            post(internal_announce_handler),
        )
        .route(
            "/internal/conferences/:room_number/recording/start",
            post(internal_recording_start_handler),
        )
        .route(
            "/internal/conferences/:room_number/recording/stop",
            post(internal_recording_stop_handler),
        )
        .route(
            "/internal/recordings/:filename",
            get(internal_download_recording_handler).delete(internal_delete_recording_handler),
        )
        .route(
            "/internal/conferences/:room_number/speaker",
            post(internal_speaker_handler),
        )
        .route(
            "/internal/conferences/:room_number/content",
            post(internal_content_handler),
        )
        .route(
            "/internal/conferences/:room_number/forwarding",
            post(internal_forwarding_handler),
        )
        .route(
            "/internal/conferences/:room_number/video",
            get(internal_video_status_handler),
        )
        .route(
            "/internal/conferences/:room_number/video/layout",
            put(internal_video_layout_handler),
        )
        .route(
            "/internal/conferences/:room_number/video/pin/:pid",
            post(internal_video_pin_handler),
        )
        .route(
            "/internal/conferences/:room_number/video/pin",
            delete(internal_video_unpin_handler),
        )
        .route(
            "/internal/conferences/:room_number/video/spotlight/:pid",
            post(internal_video_spotlight_handler),
        )
        .route(
            "/internal/conferences/:room_number/video/spotlight",
            delete(internal_video_unspotlight_handler),
        )
        .route(
            "/internal/conferences/:room_number/participants/:pid/video/:state",
            post(internal_participant_video_handler),
        )
        .route(
            "/internal/conferences/:room_number/video/content/stop",
            post(internal_content_stop_handler),
        )
        .route(
            "/internal/conferences/:room_number/video/content-layout",
            put(internal_content_layout_handler),
        )
        .route(
            "/internal/conferences/:room_number/participants/:pid/stage",
            post(internal_stage_on_handler).delete(internal_stage_off_handler),
        )
        .route(
            "/internal/conferences/:room_number/participants/:pid/hand",
            delete(internal_lower_hand_handler),
        )
        .route(
            "/internal/conferences/:room_number/hands",
            delete(internal_lower_hands_handler),
        )
        .route(
            "/internal/conferences/:room_number/webinar/live",
            post(internal_webinar_live_handler),
        )
        .route(
            "/internal/conferences/:room_number/webinar/end",
            post(internal_webinar_end_handler),
        )
        .route(
            "/internal/conferences/:room_number/webinar/stage-news",
            post(internal_stage_news_handler),
        )
        .route(
            "/internal/conferences/:room_number/webinar/edge-news",
            post(internal_edge_news_handler),
        )
        .route(
            "/internal/conferences/:room_number/webinar/qa-news",
            post(internal_qa_news_handler),
        )
        // An edge's moderation and moderator views, done on its stage
        // (`docs/WEBINARS.md` §2.7): the public handlers, which find no
        // stage above the stage and so never forward again.
        .route(
            "/internal/conferences/:room_number/questions",
            get(list_questions_handler),
        )
        .route(
            "/internal/conferences/:room_number/questions/:qid/publish",
            post(publish_question_handler),
        )
        .route(
            "/internal/conferences/:room_number/questions/:qid/answer",
            post(answer_question_handler),
        )
        .route(
            "/internal/conferences/:room_number/questions/:qid/dismiss",
            post(dismiss_question_handler),
        )
        .route(
            "/internal/conferences/:room_number/chat",
            get(list_chat_handler),
        )
        .route(
            "/internal/conferences/:room_number/chat/:mid/delete",
            post(delete_chat_handler),
        )
        .route(
            "/internal/conferences/:room_number/participants/:pid/chat-mute",
            post(mute_chat_handler),
        )
        .route_layer(middleware::from_fn_with_state(
            state,
            require_cluster_secret,
        ))
}

/// Gate for `/internal` routes: a bearer token equal to `cluster.secret`.
/// Without a configured secret the routes do not exist.
async fn require_cluster_secret(
    State(state): State<Arc<ConferenceState>>,
    req: axum::extract::Request,
    next: middleware::Next,
) -> axum::response::Response {
    let Some(secret) = state.cluster_secret.as_deref() else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": "Cross-node control is disabled on this node (cluster.secret not set)"
            })),
        )
            .into_response();
    };
    let presented = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    if !fcp_core::secret::constant_time_eq(presented, secret) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "Invalid cluster secret" })),
        )
            .into_response();
    }
    next.run(req).await
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
            protected_router().layer(middleware::from_fn(move |req, next| {
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
            info!(rps = rps, "HTTP API per-IP rate limiting enabled");
            let rate_limiter = Arc::new(PerIpRateLimiter::new(rps));
            protected = protected.layer(middleware::from_fn(move |req, next| {
                let limiter = Arc::clone(&rate_limiter);
                rate_limit_middleware(limiter, req, next)
            }));
        }
    }

    let max_body_size = state.max_body_size_bytes;
    let cors = tower_http::cors::CorsLayer::new()
        .allow_origin(tower_http::cors::Any)
        .allow_methods([
            axum::http::Method::GET,
            axum::http::Method::POST,
            axum::http::Method::PUT,
            axum::http::Method::PATCH,
            axum::http::Method::DELETE,
            axum::http::Method::OPTIONS,
        ])
        .allow_headers(tower_http::cors::Any);
    let app = public_router()
        .merge(protected)
        .merge(internal_router(Arc::clone(&state)))
        .with_state(state)
        .layer(DefaultBodyLimit::max(max_body_size))
        .layer(cors);

    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    info!(%bind_addr, "HTTP API server started");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}

// =========================================================================
// Health & Status
// =========================================================================

async fn health_handler(
    State(state): State<Arc<ConferenceState>>,
) -> (StatusCode, Json<serde_json::Value>) {
    let mut checks = serde_json::Map::new();
    let mut healthy = true;

    // Snapshot config values before any awaits
    let rec_dir = state
        .conference_handler
        .conference_config
        .read()
        .recordings_dir
        .clone();

    // Check database connectivity (use nil tenant for a lightweight probe)
    let db_ok = state
        .storage
        .list_meetings(&state.default_tenant_id)
        .await
        .is_ok();
    checks.insert(
        "database".into(),
        serde_json::json!(if db_ok { "ok" } else { "error" }),
    );
    if !db_ok {
        healthy = false;
    }

    // Check conference bridge status
    let sessions = state.conference_handler.session_count();
    let rooms = state.bridge.list_rooms();
    checks.insert("active_rooms".into(), serde_json::json!(rooms.len()));
    checks.insert("active_sessions".into(), serde_json::json!(sessions));

    // Check recordings directory is accessible
    let disk_ok = tokio::fs::metadata(&rec_dir).await.is_ok();
    checks.insert(
        "recordings_dir".into(),
        serde_json::json!(if disk_ok { "ok" } else { "missing" }),
    );

    let status = if healthy { "healthy" } else { "degraded" };
    let code = if healthy {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    (
        code,
        Json(serde_json::json!({
            "status": status,
            "service": "fcp-conference",
            "checks": checks,
        })),
    )
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
        "draining": state.conference_handler.is_draining(),
        "callers": state.conference_handler.node_callers(),
        "drain_wait_secs": state.drain.wait_secs(),
        "video_capacity": state
            .conference_handler
            .video_capacity()
            .map(|c| c.json())
            .unwrap_or(serde_json::Value::Null),
        // Whether this node's view of the cluster looks whole
        // (docs/MULTI_REGION_FOLLOWUPS.md §2.3).
        "replica": state.replica.json(),
        "cluster_view": state.cluster_view(),
        // Which release every node runs (docs/BACKUP_AND_RESTORE.md §5).
        "cluster_versions": state.cluster_versions(),
    }))
}

/// Whether this node should be sent callers: `503` while it is draining,
/// so a load balancer or a placement rule stops choosing it before it
/// stops taking them (`docs/CAPACITY_AND_SURVIVABILITY.md` §6).
/// `/health` keeps answering `200`, so a liveness probe does not kill the
/// process mid-drain — or mid-migration.
async fn ready_handler(State(state): State<Arc<ConferenceState>>) -> impl IntoResponse {
    drain_answer(fcp_sip::drain::ready_body(&state.drain).await)
}

/// Start a drain: this node takes no new callers, moves the ones it has
/// to other nodes (`docs/DISTRIBUTED_CONFERENCING.md` phase 5), waits for
/// whoever is left until `drain_wait_secs`, and hangs them up saying why.
/// The node keeps running — a drain is not a shutdown.
async fn start_drain_handler(State(state): State<Arc<ConferenceState>>) -> impl IntoResponse {
    drain_answer(fcp_sip::drain::start_body(&state.drain).await)
}

/// Call a drain off: the node takes callers again, and its rooms — which
/// the drain never tore down — carry on.
async fn stop_drain_handler(State(state): State<Arc<ConferenceState>>) -> impl IntoResponse {
    drain_answer(fcp_sip::drain::stop_body(&state.drain).await)
}

/// What the shared drain answered, as this router's response.
fn drain_answer((code, body): (u16, serde_json::Value)) -> impl IntoResponse {
    let code = StatusCode::from_u16(code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (code, Json(body))
}

async fn metrics_handler(State(state): State<Arc<ConferenceState>>) -> impl IntoResponse {
    let uptime = state.start_time.elapsed();
    let call_stats = state.call_manager.stats();
    let active_rooms = state.conference_handler.active_rooms();
    let total_participants: usize = active_rooms.iter().map(|(_, count)| count).sum();
    let active_sessions = state.conference_handler.session_count();
    let audio = state.conference_handler.aggregate_audio_stats();

    let events_dropped = state.events_dropped.load(Ordering::Relaxed);
    let latency = &state.conference_handler.handler_latency;

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
         fcp_conference_uptime_seconds {}\n\
         # HELP fcp_conference_rtp_packets_received_total RTP packets received across all sessions\n\
         # TYPE fcp_conference_rtp_packets_received_total counter\n\
         fcp_conference_rtp_packets_received_total {}\n\
         # HELP fcp_conference_rtp_packets_decoded_total RTP packets successfully decoded\n\
         # TYPE fcp_conference_rtp_packets_decoded_total counter\n\
         fcp_conference_rtp_packets_decoded_total {}\n\
         # HELP fcp_conference_rtp_decode_errors_total RTP packet decode errors\n\
         # TYPE fcp_conference_rtp_decode_errors_total counter\n\
         fcp_conference_rtp_decode_errors_total {}\n\
         # HELP fcp_conference_rtp_packets_sent_total RTP packets sent across all sessions\n\
         # TYPE fcp_conference_rtp_packets_sent_total counter\n\
         fcp_conference_rtp_packets_sent_total {}\n\
         # HELP fcp_conference_rtp_encode_errors_total RTP packet encode errors\n\
         # TYPE fcp_conference_rtp_encode_errors_total counter\n\
         fcp_conference_rtp_encode_errors_total {}\n\
         # HELP fcp_conference_dtmf_events_total DTMF events detected across all sessions\n\
         # TYPE fcp_conference_dtmf_events_total counter\n\
         fcp_conference_dtmf_events_total {}\n\
         # HELP fcp_conference_events_dropped_total Events dropped by circuit breaker\n\
         # TYPE fcp_conference_events_dropped_total counter\n\
         fcp_conference_events_dropped_total {}\n\
         # HELP fcp_conference_handler_invite_total INVITE handler invocations\n\
         # TYPE fcp_conference_handler_invite_total counter\n\
         fcp_conference_handler_invite_total {}\n\
         # HELP fcp_conference_handler_invite_duration_us Total INVITE handler duration (microseconds)\n\
         # TYPE fcp_conference_handler_invite_duration_us counter\n\
         fcp_conference_handler_invite_duration_us {}\n\
         # HELP fcp_conference_handler_reinvite_total re-INVITE handler invocations\n\
         # TYPE fcp_conference_handler_reinvite_total counter\n\
         fcp_conference_handler_reinvite_total {}\n\
         # HELP fcp_conference_handler_bye_total BYE handler invocations\n\
         # TYPE fcp_conference_handler_bye_total counter\n\
         fcp_conference_handler_bye_total {}\n\
         # HELP fcp_conference_handler_subscribe_total SUBSCRIBE handler invocations\n\
         # TYPE fcp_conference_handler_subscribe_total counter\n\
         fcp_conference_handler_subscribe_total {}\n\
         # HELP fcp_conference_handler_refer_total REFER handler invocations\n\
         # TYPE fcp_conference_handler_refer_total counter\n\
         fcp_conference_handler_refer_total {}\n\
         # HELP fcp_conference_active_subscriptions Active RFC 4575 event subscriptions\n\
         # TYPE fcp_conference_active_subscriptions gauge\n\
         fcp_conference_active_subscriptions {}\n",
        active_rooms.len(),
        total_participants,
        active_sessions,
        call_stats.total,
        uptime.as_secs(),
        audio.packets_received,
        audio.packets_decoded,
        audio.decode_errors,
        audio.packets_sent,
        audio.encode_errors,
        audio.dtmf_events,
        events_dropped,
        latency.invite_count.load(Ordering::Relaxed),
        latency.invite_total_us.load(Ordering::Relaxed),
        latency.reinvite_count.load(Ordering::Relaxed),
        latency.bye_count.load(Ordering::Relaxed),
        latency.subscribe_count.load(Ordering::Relaxed),
        latency.refer_count.load(Ordering::Relaxed),
        state.conference_handler.subscription_count(),
    );
    let mut body = body;
    if let Some(view) = state.cluster_view() {
        body.push_str(&view.prometheus(""));
    }
    if let Some(versions) = state.cluster_versions() {
        body.push_str(&versions.prometheus(""));
    }
    body.push_str(&state.replica.prometheus(""));

    (
        StatusCode::OK,
        [("content-type", "text/plain; version=0.0.4; charset=utf-8")],
        body,
    )
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
    #[serde(default)]
    owner_id: Option<String>,
    #[serde(default)]
    pin_prompt_sound: Option<String>,
    #[serde(default)]
    invalid_pin_sound: Option<String>,
    #[serde(default)]
    moh_sound: Option<String>,
    #[serde(default)]
    pin_prompt_tts: Option<String>,
    #[serde(default)]
    invalid_pin_tts: Option<String>,
    #[serde(default)]
    welcome_tts: Option<String>,
    #[serde(default = "default_true")]
    play_entry_tone: bool,
    #[serde(default = "default_true")]
    play_exit_tone: bool,
    #[serde(default)]
    join_sound: Option<String>,
    #[serde(default)]
    exit_sound: Option<String>,
    #[serde(default)]
    video_enabled: bool,
    #[serde(default = "fcp_core::meeting::default_video_layout")]
    video_layout: String,
    #[serde(default = "fcp_core::meeting::default_video_max_participants")]
    video_max_participants: u32,
    #[serde(default = "fcp_core::meeting::default_video_max_resolution")]
    video_max_resolution: String,
    #[serde(default = "fcp_core::meeting::default_video_fps")]
    video_fps: u32,
    #[serde(default)]
    video_exclude_self: bool,
    #[serde(default = "fcp_core::meeting::default_video_codecs")]
    video_codecs: Vec<fcp_core::VideoCodec>,
    #[serde(default = "fcp_core::meeting::default_video_passthrough")]
    video_passthrough: String,
    #[serde(default = "fcp_core::meeting::default_video_content_share")]
    video_content_share: String,
    #[serde(default = "fcp_core::meeting::default_video_content_layout")]
    video_content_layout: String,
    #[serde(default = "fcp_core::meeting::default_video_mode")]
    video_mode: String,
    /// `meeting` or `webinar`; a webinar's panelists are named through
    /// the admin API, which knows the tenant's users.
    #[serde(default = "fcp_core::meeting::default_meeting_kind")]
    kind: String,
    #[serde(default)]
    panelist_pin: Option<String>,
}

fn default_true() -> bool {
    true
}

/// Pagination query parameters for list endpoints.
#[derive(Deserialize)]
struct PaginationParams {
    #[serde(default)]
    offset: usize,
    #[serde(default = "default_page_limit")]
    limit: usize,
}

fn default_page_limit() -> usize {
    100
}

async fn list_meetings_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Query(pagination): Query<PaginationParams>,
) -> Json<serde_json::Value> {
    use fcp_core::storage::ConferenceStore;

    let tenant_id = match extract_tenant_id(&headers, &state).await {
        Ok(tenant_id) => tenant_id,
        Err((_, body)) => return body,
    };
    match state.storage.list_meetings(&tenant_id).await {
        Ok(meetings) => {
            let total = meetings.len();
            let page: Vec<serde_json::Value> = meetings
                .iter()
                .skip(pagination.offset)
                .take(pagination.limit)
                .map(meeting_to_json)
                .collect();
            Json(serde_json::json!({
                "meetings": page,
                "total": total,
                "offset": pagination.offset,
                "limit": pagination.limit,
            }))
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

    let tenant_id = match extract_tenant_id(&headers, &state).await {
        Ok(tenant_id) => tenant_id,
        Err(err) => return err,
    };

    // Validate room number
    if !crate::conference_handler::validate_room_number(&payload.room_number) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "Invalid room number: must be 1-64 chars, alphanumeric or -_.+*#"
            })),
        );
    }

    // Warn if guest and host PINs are the same (host PIN would be useless)
    if let (Some(ref gp), Some(ref hp)) = (&payload.guest_pin, &payload.host_pin) {
        if gp == hp {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "Guest PIN and Host PIN must be different. When they are the same, \
                              no one can authenticate as host, which breaks wait-for-moderator."
                })),
            );
        }
    }

    match room_number_exists_in_tenant(&state, &tenant_id, &payload.room_number).await {
        Ok(true) => {
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
        Ok(false) => {}
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
        owner_id: payload.owner_id,
        pin_prompt_sound: payload.pin_prompt_sound,
        invalid_pin_sound: payload.invalid_pin_sound,
        moh_sound: payload.moh_sound,
        pin_prompt_tts: payload.pin_prompt_tts,
        invalid_pin_tts: payload.invalid_pin_tts,
        welcome_tts: payload.welcome_tts,
        play_entry_tone: payload.play_entry_tone,
        play_exit_tone: payload.play_exit_tone,
        join_sound: payload.join_sound,
        exit_sound: payload.exit_sound,
        video_enabled: payload.video_enabled,
        video_layout: payload.video_layout,
        video_max_participants: payload.video_max_participants,
        video_max_resolution: payload.video_max_resolution,
        video_fps: payload.video_fps,
        video_exclude_self: payload.video_exclude_self,
        video_codecs: payload.video_codecs,
        video_passthrough: payload.video_passthrough,
        video_content_share: payload.video_content_share,
        video_content_layout: payload.video_content_layout,
        video_mode: payload.video_mode,
        kind: payload.kind,
        panelist_pin: payload.panelist_pin,
        panelists: Vec::new(),
        qa: None,
        chat: None,
        qa_anonymous: false,
        registration: false,
        streaming: None,
        created_at: now,
        updated_at: now,
    };
    if let Err(msg) = meeting
        .validate_video_settings()
        .and_then(|()| meeting.validate_roles())
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": msg })),
        );
    }

    match state.storage.put_meeting(meeting.clone()).await {
        Ok(()) => {
            info!(action = "create_meeting", room = %meeting.room_number, name = %meeting.name, "AUDIT: meeting created via API");
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
    headers: HeaderMap,
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

    let expected_tenant = match extract_tenant_id(&headers, &state).await {
        Ok(tenant_id) => tenant_id,
        Err(err) => return err,
    };
    match state.storage.get_meeting(&meeting_id).await {
        Ok(Some(meeting)) if meeting.tenant_id == expected_tenant => {
            (StatusCode::OK, Json(meeting_to_json(&meeting)))
        }
        Ok(Some(_)) | Ok(None) => (
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
    owner_id: Option<Option<String>>,
    pin_prompt_sound: Option<Option<String>>,
    invalid_pin_sound: Option<Option<String>>,
    moh_sound: Option<Option<String>>,
    pin_prompt_tts: Option<Option<String>>,
    invalid_pin_tts: Option<Option<String>>,
    welcome_tts: Option<Option<String>>,
    play_entry_tone: Option<bool>,
    play_exit_tone: Option<bool>,
    join_sound: Option<Option<String>>,
    exit_sound: Option<Option<String>>,
    video_enabled: Option<bool>,
    video_layout: Option<String>,
    video_max_participants: Option<u32>,
    video_max_resolution: Option<String>,
    video_fps: Option<u32>,
    video_exclude_self: Option<bool>,
    video_codecs: Option<Vec<fcp_core::VideoCodec>>,
    video_content_share: Option<String>,
    video_content_layout: Option<String>,
    video_mode: Option<String>,
    kind: Option<String>,
    panelist_pin: Option<String>,
}

async fn update_meeting_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
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

    let expected_tenant = match extract_tenant_id(&headers, &state).await {
        Ok(tenant_id) => tenant_id,
        Err(err) => return err,
    };
    let mut meeting = match state.storage.get_meeting(&meeting_id).await {
        Ok(Some(m)) if m.tenant_id == expected_tenant => m,
        Ok(Some(_)) | Ok(None) => {
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
    if let Some(v) = payload.owner_id {
        meeting.owner_id = v;
        changed_fields.push("owner_id".to_string());
    }
    if let Some(v) = payload.pin_prompt_sound {
        meeting.pin_prompt_sound = v;
        changed_fields.push("pin_prompt_sound".to_string());
    }
    if let Some(v) = payload.invalid_pin_sound {
        meeting.invalid_pin_sound = v;
        changed_fields.push("invalid_pin_sound".to_string());
    }
    if let Some(v) = payload.moh_sound {
        meeting.moh_sound = v;
        changed_fields.push("moh_sound".to_string());
    }
    if let Some(v) = payload.pin_prompt_tts {
        meeting.pin_prompt_tts = v;
        changed_fields.push("pin_prompt_tts".to_string());
    }
    if let Some(v) = payload.invalid_pin_tts {
        meeting.invalid_pin_tts = v;
        changed_fields.push("invalid_pin_tts".to_string());
    }
    if let Some(v) = payload.welcome_tts {
        meeting.welcome_tts = v;
        changed_fields.push("welcome_tts".to_string());
    }
    if let Some(v) = payload.play_entry_tone {
        meeting.play_entry_tone = v;
        changed_fields.push("play_entry_tone".to_string());
    }
    if let Some(v) = payload.play_exit_tone {
        meeting.play_exit_tone = v;
        changed_fields.push("play_exit_tone".to_string());
    }
    if let Some(v) = payload.join_sound {
        meeting.join_sound = v;
        changed_fields.push("join_sound".to_string());
    }
    if let Some(v) = payload.exit_sound {
        meeting.exit_sound = v;
        changed_fields.push("exit_sound".to_string());
    }
    if let Some(v) = payload.video_enabled {
        meeting.video_enabled = v;
        changed_fields.push("video_enabled".to_string());
    }
    if let Some(v) = payload.video_layout {
        meeting.video_layout = v;
        changed_fields.push("video_layout".to_string());
    }
    if let Some(v) = payload.video_max_participants {
        meeting.video_max_participants = v;
        changed_fields.push("video_max_participants".to_string());
    }
    if let Some(v) = payload.video_max_resolution {
        meeting.video_max_resolution = v;
        changed_fields.push("video_max_resolution".to_string());
    }
    if let Some(v) = payload.video_fps {
        meeting.video_fps = v;
        changed_fields.push("video_fps".to_string());
    }
    if let Some(v) = payload.video_exclude_self {
        meeting.video_exclude_self = v;
        changed_fields.push("video_exclude_self".to_string());
    }
    if let Some(v) = payload.video_codecs {
        meeting.video_codecs = v;
        changed_fields.push("video_codecs".to_string());
    }
    if let Some(v) = payload.video_content_share {
        meeting.video_content_share = v;
        changed_fields.push("video_content_share".to_string());
    }
    if let Some(v) = payload.video_content_layout {
        meeting.video_content_layout = v;
        changed_fields.push("video_content_layout".to_string());
    }
    if let Some(v) = payload.video_mode {
        meeting.video_mode = v;
        changed_fields.push("video_mode".to_string());
    }
    if let Some(v) = payload.kind {
        meeting.kind = v;
        changed_fields.push("kind".to_string());
    }
    if let Some(v) = payload.panelist_pin {
        meeting.panelist_pin = Some(v);
        changed_fields.push("panelist_pin".to_string());
    }
    if let Err(msg) = meeting
        .validate_video_settings()
        .and_then(|()| meeting.validate_roles())
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": msg })),
        );
    }
    meeting.updated_at = SystemTime::now();

    match state.storage.put_meeting(meeting.clone()).await {
        Ok(()) => {
            info!(action = "update_meeting", room = %meeting.room_number, fields = ?changed_fields, "AUDIT: meeting updated via API");
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
    headers: HeaderMap,
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

    let expected_tenant = match extract_tenant_id(&headers, &state).await {
        Ok(tenant_id) => tenant_id,
        Err(err) => return err,
    };
    let meeting_info = match state.storage.get_meeting(&meeting_id).await {
        Ok(Some(meeting)) if meeting.tenant_id == expected_tenant => Some(meeting),
        Ok(Some(_)) | Ok(None) => {
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

    match state.storage.delete_meeting(&meeting_id).await {
        Ok(()) => {
            info!(action = "delete_meeting", meeting_id = %id, "AUDIT: meeting deleted via API");
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
    headers: HeaderMap,
) -> Json<serde_json::Value> {
    // A request that names a tenant sees that tenant's rooms; one that
    // names none (the node's own operator) sees every room, each with
    // its tenant.
    let only = headers
        .get(PLATFORM_TENANT_HEADER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<TenantId>().ok());
    let rooms = state.conference_handler.active_rooms();
    let list: Vec<serde_json::Value> = rooms
        .iter()
        .filter(|(key, _)| only.is_none_or(|t| key.tenant == t))
        .map(|(key, count)| {
            serde_json::json!({
                "room_number": key.room,
                "tenant_id": key.tenant,
                "participants": count,
            })
        })
        .collect();

    Json(serde_json::json!({ "conferences": list }))
}

async fn get_conference_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path(room_number): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = match room_key(&state, &headers, &room_number).await {
        Ok(key) => key,
        Err(err) => return err,
    };
    match state.bridge.get_room(&key) {
        Ok(room) => {
            let participants = room.participants();
            let trunks = state.conference_handler.room_trunk_peers(&key);
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "room_number": room_number,
                    "participant_count": room.participant_count(),
                    "participants": participants,
                    "is_locked": room.is_locked(),
                    "is_recording": room.is_recording(),
                    // The registry's view of what is being recorded here,
                    // so a caller can follow it to the file.
                    "recording": state
                        .conference_handler
                        .room_recording(&key)
                        .as_ref()
                        .map(recording_json),
                    "node_id": state.conference_handler.node_id(),
                    "trunks": trunks,
                    // `stage` or `edge` of a webinar across nodes
                    // (`docs/WEBINARS.md` §2.7).
                    "webinar_node": state.conference_handler.webinar_node(&key),
                    "video": crate::video_bridge::room_video_json(&room),
                    "audience": crate::conference_handler::audience_json(&room),
                })),
            )
        }
        Err(_) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Conference room not active" })),
        ),
    }
}

#[derive(Deserialize)]
struct DialOutRequest {
    destination: String,
    /// TTS text to play to the callee before connecting (e.g. "You are being invited to Jerry's conference")
    #[serde(default)]
    greeting_tts: Option<String>,
    /// Name of an audio prompt to play as greeting (looked up from prompt store)
    #[serde(default)]
    greeting_prompt: Option<String>,
}

async fn dial_out_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path(room_number): Path<String>,
    Json(payload): Json<DialOutRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = match room_key(&state, &headers, &room_number).await {
        Ok(key) => key,
        Err(err) => return err,
    };
    let raw = payload.destination.trim().to_string();
    if raw.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Destination is required" })),
        );
    }

    // Normalize the destination into a proper SIP or tel URI.
    // Accepts: sip:user@host, tel:+15551234, user@host, +15551234, 1001
    let dest = if raw.starts_with("sip:") || raw.starts_with("tel:") {
        // Already a proper URI
        raw
    } else if raw.contains('@') {
        // user@domain → sip:user@domain
        format!("sip:{}", raw)
    } else {
        // Bare extension or phone number → sip:ext@local_domain
        // Extract the domain from the conference server's local SIP address
        let local_addr = &state.conference_handler.local_addr;
        let domain = local_addr.split(':').next().unwrap_or(local_addr);
        format!("sip:{}@{}", raw, domain)
    };

    match state
        .conference_handler
        .dial_out(
            &key,
            &dest,
            payload.greeting_tts.as_deref(),
            payload.greeting_prompt.as_deref(),
        )
        .await
    {
        Ok(participant_id) => {
            info!(
                action = "dial_out",
                room = %room_number,
                destination = %dest,
                participant = %participant_id,
                "AUDIT: dial-out initiated via API"
            );
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "connected",
                    "participant_id": participant_id,
                    "destination": dest,
                    "room_number": room_number,
                })),
            )
        }
        Err(e) => {
            warn!(room = %room_number, destination = %dest, error = %e, "Dial-out failed");
            let msg = format!("{}", e);
            let code = if msg.contains("not active") {
                StatusCode::NOT_FOUND
            } else if msg.contains("full") || msg.contains("locked") {
                StatusCode::CONFLICT
            } else if msg.contains("timed out") || msg.contains("rejected") {
                StatusCode::GATEWAY_TIMEOUT
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            (code, Json(serde_json::json!({ "error": msg })))
        }
    }
}

async fn start_recording_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path(room_number): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    if !is_valid_identifier(&room_number) {
        return invalid_identifier();
    }
    let tenant_id = match extract_tenant_id(&headers, &state).await {
        Ok(tenant_id) => tenant_id,
        Err(err) => return err,
    };
    let key = RoomKey::new(tenant_id, room_number.to_string());
    // One recording per room, on the lowest-id hosting node — the rule
    // the trunks are dialled by. Without it a room hosted on two nodes,
    // started through each of them, would be recorded twice.
    if let Some(peer) = lower_hosting_node(&state, &key).await {
        if let Some(control) = &state.control {
            if let Some(outcome) = control
                .send_to_node(
                    &peer,
                    &tenant_id.to_string(),
                    reqwest::Method::POST,
                    &format!("/conferences/{room_number}/recording/start"),
                    None,
                )
                .await
            {
                // A peer that answered — even to refuse — has the say.
                // One that could not be reached does not stop the room
                // being recorded here.
                if let Some(status) = outcome.status {
                    let mut body = outcome.body;
                    if let Some(map) = body.as_object_mut() {
                        map.insert("node".into(), outcome.node.into());
                    }
                    return (
                        StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY),
                        Json(body),
                    );
                }
                warn!(room = %room_number, peer = %peer, "The lower-id host is unreachable; recording here instead");
            }
        }
    }
    let local = local_recording_start(&state, &key).await;
    let local_status = local.as_ref().err().map(|(s, _)| *s);
    let (status, body) = wherever_it_is(
        &state,
        &key,
        local,
        reqwest::Method::POST,
        &format!("/conferences/{room_number}/recording/start"),
        None,
    )
    .await;
    // A room that is already being recorded here is a conflict, not a
    // miss: `wherever_it_is` only asks the peers on a 404.
    match local_status {
        Some(local_status) if local_status != StatusCode::NOT_FOUND => (local_status, body),
        _ => (status, body),
    }
}

/// The hosting node whose id sorts below this one's, when there is one:
/// where a room's single recording belongs.
async fn lower_hosting_node(state: &ConferenceState, key: &RoomKey) -> Option<String> {
    let room_number = key.room.as_str();
    let tenant_id = &key.tenant;
    let control = state.control.as_ref()?;
    let peers = match control
        .hosting_peers(&tenant_id.to_string(), room_number)
        .await
    {
        Ok(peers) => peers,
        Err(e) => {
            warn!(room = %room_number, error = %e, "Could not read the registry; recording where the request landed");
            return None;
        }
    };
    // `hosting_peers` sorts by node id, so the first is the lowest.
    peers
        .into_iter()
        .next()
        .filter(|p| p.node_id.as_str() < state.conference_handler.node_id())
        .map(|p| p.node_id)
}

/// POST /conferences/{room}/recording/stop — room-wide: whichever node is
/// recording stops.
async fn stop_recording_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path(room_number): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    if !is_valid_identifier(&room_number) {
        return invalid_identifier();
    }
    let tenant_id = match extract_tenant_id(&headers, &state).await {
        Ok(tenant_id) => tenant_id,
        Err(err) => return err,
    };
    let key = RoomKey::new(tenant_id, room_number.to_string());
    let local = local_recording_stop(&state, &key).await;
    let local_status = local.as_ref().err().map(|(s, _)| *s);
    let (ok, body) = room_wide(
        &state,
        &key,
        local,
        reqwest::Method::POST,
        &format!("/conferences/{room_number}/recording/stop"),
        None,
    )
    .await;
    if !ok {
        return (local_status.unwrap_or(StatusCode::NOT_FOUND), Json(body));
    }
    (StatusCode::OK, Json(body))
}

// =========================================================================
// Participant Details & Stats
// =========================================================================

async fn get_participant_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path((room_number, pid)): Path<(String, String)>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = match room_key(&state, &headers, &room_number).await {
        Ok(key) => key,
        Err(err) => return err,
    };
    if !is_valid_identifier(&room_number) || !is_valid_identifier(&pid) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Invalid identifier" })),
        );
    }
    match state.conference_handler.get_participant_details(&key, &pid) {
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

async fn list_waiting_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path(room_number): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = match room_key(&state, &headers, &room_number).await {
        Ok(key) => key,
        Err(err) => return err,
    };
    match state.conference_handler.list_waiting(&key) {
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
    headers: HeaderMap,
    Path((room_number, pid)): Path<(String, String)>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = match room_key(&state, &headers, &room_number).await {
        Ok(key) => key,
        Err(err) => return err,
    };
    if !is_valid_identifier(&room_number) || !is_valid_identifier(&pid) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Invalid identifier" })),
        );
    }
    match state.conference_handler.promote_participant(&key, &pid) {
        Ok(()) => {
            info!(action = "promote", room = %room_number, participant = %pid, "AUDIT: participant promoted via API");
            (
                StatusCode::OK,
                Json(serde_json::json!({ "status": "promoted", "participant": pid })),
            )
        }
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": format!("{}", e) })),
        ),
    }
}

/// The answer to a hand or stage operation.
fn stage_answer(
    result: Result<bool, crate::conference_handler::StageError>,
    pid: &str,
) -> (StatusCode, Json<serde_json::Value>) {
    use crate::conference_handler::StageError;
    match result {
        Ok(changed) => (
            StatusCode::OK,
            Json(serde_json::json!({ "participant": pid, "changed": changed })),
        ),
        Err(StageError::NotFound) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": StageError::NotFound.to_string() })),
        ),
        Err(e) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

/// The answer to a webinar operation that failed.
fn webinar_refusal(
    e: crate::conference_handler::WebinarError,
) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::from_u16(e.status()).unwrap_or(StatusCode::CONFLICT),
        Json(serde_json::json!({ "error": e.to_string() })),
    )
}

/// `GET /conferences/:room/webinar` — the room's sitting: its id, kind,
/// phase, when it started and went live, and how many wait in the lobby.
async fn webinar_status_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path(room_number): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = match room_key(&state, &headers, &room_number).await {
        Ok(key) => key,
        Err(err) => return err,
    };
    match state.conference_handler.sitting_json(&key) {
        Some(sitting) => (StatusCode::OK, Json(sitting)),
        None => webinar_refusal(crate::conference_handler::WebinarError::NotFound),
    }
}

/// `POST /conferences/:room/webinar/live` — a host takes the webinar live:
/// the lobby is let in and recording starts if the meeting records.
async fn webinar_live_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path(room_number): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = match room_key(&state, &headers, &room_number).await {
        Ok(key) => key,
        Err(err) => return err,
    };
    if let Some(answer) = to_the_stage(&state, &key, "live").await {
        return answer;
    }
    match state.conference_handler.go_live(&key, None) {
        Ok(changed) => {
            info!(action = "webinar_live", room = %room_number, changed, "AUDIT: webinar taken live via API");
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "changed": changed,
                    "sitting": state.conference_handler.sitting_json(&key),
                })),
            )
        }
        Err(e) => webinar_refusal(e),
    }
}

/// `POST /conferences/:room/webinar/end` — a host ends the webinar:
/// everyone hears the ending announcement and is released.
async fn webinar_end_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path(room_number): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = match room_key(&state, &headers, &room_number).await {
        Ok(key) => key,
        Err(err) => return err,
    };
    if let Some(answer) = to_the_stage(&state, &key, "end").await {
        return answer;
    }
    let sitting = state.conference_handler.sitting_json(&key);
    match state.conference_handler.end_webinar(&key, "api").await {
        Ok(released) => {
            info!(action = "webinar_end", room = %room_number, released, "AUDIT: webinar ended via API");
            (
                StatusCode::OK,
                Json(serde_json::json!({ "released": released, "sitting": sitting })),
            )
        }
        Err(e) => webinar_refusal(e),
    }
}

/// On an edge of a webinar's tree (§2.7), a webinar operation is the
/// stage's: forwarded there, its answer handed back. `None` elsewhere.
async fn to_the_stage(
    state: &ConferenceState,
    key: &RoomKey,
    op: &str,
) -> Option<(StatusCode, Json<serde_json::Value>)> {
    to_the_stage_at(
        state,
        key,
        reqwest::Method::POST,
        &format!("/conferences/{}/webinar/{op}", key.room),
        None,
    )
    .await
}

/// `method path` (with its query, and `body`) done on the stage when this
/// node is an edge of `key`'s webinar.
async fn to_the_stage_at(
    state: &ConferenceState,
    key: &RoomKey,
    method: reqwest::Method,
    path: &str,
    body: Option<serde_json::Value>,
) -> Option<(StatusCode, Json<serde_json::Value>)> {
    let stage = state.conference_handler.stage_of(key)?;
    let control = state.control.as_ref()?;
    let outcome = control
        .send_to_node(&stage, &key.tenant.to_string(), method, path, body)
        .await?;
    let status = outcome
        .status
        .and_then(|s| StatusCode::from_u16(s).ok())
        .unwrap_or(StatusCode::BAD_GATEWAY);
    let mut body = outcome.body;
    if let Some(map) = body.as_object_mut() {
        map.insert("node".into(), stage.into());
    }
    Some((status, Json(body)))
}

/// `POST /internal/conferences/:room/webinar/live` — an edge forwards a
/// request to take the webinar live to its stage.
async fn internal_webinar_live_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path(room_number): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = match room_key(&state, &headers, &room_number).await {
        Ok(key) => key,
        Err(err) => return err,
    };
    match state.conference_handler.go_live(&key, None) {
        Ok(changed) => {
            info!(action = "webinar_live", room = %room_number, changed, from = %forwarded_from(&headers), "AUDIT: webinar taken live on behalf of an edge");
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "changed": changed,
                    "sitting": state.conference_handler.sitting_json(&key),
                })),
            )
        }
        Err(e) => webinar_refusal(e),
    }
}

/// `POST /internal/conferences/:room/webinar/end` — an edge forwards a
/// request to end the webinar to its stage.
async fn internal_webinar_end_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path(room_number): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = match room_key(&state, &headers, &room_number).await {
        Ok(key) => key,
        Err(err) => return err,
    };
    let sitting = state.conference_handler.sitting_json(&key);
    match state.conference_handler.end_webinar(&key, "api").await {
        Ok(released) => {
            info!(action = "webinar_end", room = %room_number, released, from = %forwarded_from(&headers), "AUDIT: webinar ended on behalf of an edge");
            (
                StatusCode::OK,
                Json(serde_json::json!({ "released": released, "sitting": sitting })),
            )
        }
        Err(e) => webinar_refusal(e),
    }
}

/// `POST /internal/conferences/:room/webinar/stage-news` — the stage of a
/// webinar this node is an edge of says how it stands.
async fn internal_stage_news_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path(room_number): Path<String>,
    Json(news): Json<crate::conference_handler::StageNews>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = match room_key(&state, &headers, &room_number).await {
        Ok(key) => key,
        Err(err) => return err,
    };
    if state.conference_handler.apply_stage_news(&key, news) {
        (StatusCode::OK, Json(serde_json::json!({ "applied": true })))
    } else {
        (
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "error": "this node is not an edge of that stage" })),
        )
    }
}

/// `POST /internal/conferences/:room/webinar/qa-news` — the stage of a
/// webinar this node is an edge of sends its questions and chat.
async fn internal_qa_news_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path(room_number): Path<String>,
    Json(news): Json<crate::conference_handler::QaNews>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = match room_key(&state, &headers, &room_number).await {
        Ok(key) => key,
        Err(err) => return err,
    };
    if state.conference_handler.apply_qa_news(&key, news) {
        (StatusCode::OK, Json(serde_json::json!({ "applied": true })))
    } else {
        (
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "error": "this node is not an edge of that stage" })),
        )
    }
}

/// `POST /internal/conferences/:room/webinar/edge-news` — an edge of a
/// webinar whose stage is here says how it stands.
async fn internal_edge_news_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path(room_number): Path<String>,
    Json(news): Json<crate::conference_handler::EdgeNews>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = match room_key(&state, &headers, &room_number).await {
        Ok(key) => key,
        Err(err) => return err,
    };
    if state.conference_handler.apply_edge_news(&key, news) {
        (StatusCode::OK, Json(serde_json::json!({ "applied": true })))
    } else {
        (
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "error": "this node is not the stage" })),
        )
    }
}

/// `?limit=&cursor=` of the audience.
#[derive(Debug, Deserialize)]
struct AudienceQuery {
    limit: Option<usize>,
    cursor: Option<String>,
}

/// `GET /conferences/:room/audience` — a page of the room's attendees, by
/// participant id: in the room, on the stage, or in the lobby.
async fn audience_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path(room_number): Path<String>,
    Query(q): Query<AudienceQuery>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = match room_key(&state, &headers, &room_number).await {
        Ok(key) => key,
        Err(err) => return err,
    };
    let limit = q.limit.unwrap_or(100).clamp(1, 1000);
    match state.conference_handler.audience_page(
        &key,
        q.cursor.as_deref().filter(|c| !c.is_empty()),
        limit,
    ) {
        Some((data, next_cursor, total)) => (
            StatusCode::OK,
            Json(serde_json::json!({ "data": data, "next_cursor": next_cursor, "total": total })),
        ),
        None => webinar_refusal(crate::conference_handler::WebinarError::NotFound),
    }
}

/// The body of `POST /conferences/:room/hand`.
#[derive(Debug, Deserialize)]
struct OwnHandRequest {
    /// The participant's call: the call manager's or the gateway's
    /// `X-FCP-Call-Id`, or the leg's SIP Call-ID.
    call_id: String,
    raised: bool,
}

/// `POST /conferences/:room/hand` — a participant raises or lowers their
/// own hand, named by their call (their app, through the admin's
/// `/me/conferences/live/{room}/hand`).
async fn own_hand_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path(room_number): Path<String>,
    Json(req): Json<OwnHandRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = match room_key(&state, &headers, &room_number).await {
        Ok(key) => key,
        Err(err) => return err,
    };
    let Some(pid) = state
        .conference_handler
        .participant_by_call(&key, req.call_id.trim())
    else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "no participant on that call in the room" })),
        );
    };
    let result = state.conference_handler.set_hand(
        &key,
        &pid,
        req.raised,
        crate::conference_handler::HandBy::Owner,
    );
    stage_answer(result, &pid)
}

/// `DELETE /conferences/:room/participants/:pid/hand` — a host lowers one
/// hand.
async fn lower_hand_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path((room_number, pid)): Path<(String, String)>,
) -> (StatusCode, Json<serde_json::Value>) {
    lower_hand(state, headers, room_number, pid, true).await
}

/// `DELETE /internal/conferences/:room/participants/:pid/hand` — a peer's
/// host lowers the hand of an attendee here.
async fn internal_lower_hand_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path((room_number, pid)): Path<(String, String)>,
) -> (StatusCode, Json<serde_json::Value>) {
    lower_hand(state, headers, room_number, pid, false).await
}

async fn lower_hand(
    state: Arc<ConferenceState>,
    headers: HeaderMap,
    room_number: String,
    pid: String,
    forward: bool,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = match room_key(&state, &headers, &room_number).await {
        Ok(key) => key,
        Err(err) => return err,
    };
    let result = state.conference_handler.set_hand(
        &key,
        &pid,
        false,
        crate::conference_handler::HandBy::Host,
    );
    if matches!(result, Ok(true)) {
        info!(action = "lower_hand", room = %room_number, participant = %pid, "AUDIT: hand lowered via API");
    }
    if forward && matches!(result, Err(crate::conference_handler::StageError::NotFound)) {
        return wherever_it_is(
            &state,
            &key,
            Err((StatusCode::NOT_FOUND, result.unwrap_err().to_string())),
            reqwest::Method::DELETE,
            &format!("/conferences/{room_number}/participants/{pid}/hand"),
            None,
        )
        .await;
    }
    stage_answer(result, &pid)
}

/// `DELETE /conferences/:room/hands` — a host lowers every hand.
async fn lower_hands_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path(room_number): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = match room_key(&state, &headers, &room_number).await {
        Ok(key) => key,
        Err(err) => return err,
    };
    let lowered = state.conference_handler.lower_hands(&key);
    info!(action = "lower_hands", room = %room_number, lowered, "AUDIT: hands lowered via API");
    // Every node of a webinar's tree lowers its own.
    let (_, body) = room_wide(
        &state,
        &key,
        Ok(serde_json::json!({ "lowered": lowered })),
        reqwest::Method::DELETE,
        &format!("/conferences/{room_number}/hands"),
        None,
    )
    .await;
    (StatusCode::OK, Json(body))
}

/// `DELETE /internal/conferences/:room/hands` — a peer's host lowers every
/// hand here.
async fn internal_lower_hands_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path(room_number): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = match room_key(&state, &headers, &room_number).await {
        Ok(key) => key,
        Err(err) => return err,
    };
    let lowered = state.conference_handler.lower_hands(&key);
    (
        StatusCode::OK,
        Json(serde_json::json!({ "lowered": lowered })),
    )
}

/// `POST /conferences/:room/participants/:pid/stage` — bring a webinar's
/// attendee to the stage.
async fn stage_on_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path((room_number, pid)): Path<(String, String)>,
) -> (StatusCode, Json<serde_json::Value>) {
    stage_handler(state, headers, room_number, pid, true, true).await
}

/// `DELETE /conferences/:room/participants/:pid/stage` — send them back.
async fn stage_off_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path((room_number, pid)): Path<(String, String)>,
) -> (StatusCode, Json<serde_json::Value>) {
    stage_handler(state, headers, room_number, pid, false, true).await
}

async fn stage_handler(
    state: Arc<ConferenceState>,
    headers: HeaderMap,
    room_number: String,
    pid: String,
    on: bool,
    forward: bool,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = match room_key(&state, &headers, &room_number).await {
        Ok(key) => key,
        Err(err) => return err,
    };
    let result = state.conference_handler.set_stage(&key, &pid, on);
    if matches!(result, Ok(true)) {
        info!(action = if on { "stage_on" } else { "stage_off" }, room = %room_number, participant = %pid, "AUDIT: the stage changed via API");
    }
    // An attendee on another node of a webinar's tree (§2.7) is brought up
    // there.
    if forward && matches!(result, Err(crate::conference_handler::StageError::NotFound)) {
        let method = if on {
            reqwest::Method::POST
        } else {
            reqwest::Method::DELETE
        };
        return wherever_it_is(
            &state,
            &key,
            Err((StatusCode::NOT_FOUND, result.unwrap_err().to_string())),
            method,
            &format!("/conferences/{room_number}/participants/{pid}/stage"),
            None,
        )
        .await;
    }
    stage_answer(result, &pid)
}

/// `POST /internal/conferences/:room/participants/:pid/stage` — a peer's
/// host brings an attendee here to the stage.
async fn internal_stage_on_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path((room_number, pid)): Path<(String, String)>,
) -> (StatusCode, Json<serde_json::Value>) {
    stage_handler(state, headers, room_number, pid, true, false).await
}

/// `DELETE /internal/conferences/:room/participants/:pid/stage`.
async fn internal_stage_off_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path((room_number, pid)): Path<(String, String)>,
) -> (StatusCode, Json<serde_json::Value>) {
    stage_handler(state, headers, room_number, pid, false, false).await
}

/// The answer for a question operation's error.
fn qa_error(e: crate::conference_handler::QaError) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::from_u16(e.status()).unwrap_or(StatusCode::BAD_REQUEST),
        Json(serde_json::json!({ "error": e.to_string() })),
    )
}

/// The participant on `call_id` in `key`'s room, or the 404 to answer.
fn participant_of_call(
    state: &ConferenceState,
    key: &crate::rooms::RoomKey,
    call_id: &str,
) -> Result<String, (StatusCode, Json<serde_json::Value>)> {
    state
        .conference_handler
        .participant_by_call(key, call_id.trim())
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "no participant on that call in the room" })),
            )
        })
}

#[derive(Debug, Deserialize)]
struct QuestionsQuery {
    /// The call to see the questions as; none sees them as a host does.
    call_id: Option<String>,
}

/// `GET /conferences/:room/questions[?call_id=]` — the room's questions,
/// as the participant on that call sees them or as a host does.
async fn list_questions_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path(room_number): Path<String>,
    Query(q): Query<QuestionsQuery>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = match room_key(&state, &headers, &room_number).await {
        Ok(key) => key,
        Err(err) => return err,
    };
    let pid = match q.call_id.as_deref().filter(|c| !c.trim().is_empty()) {
        Some(call_id) => match participant_of_call(&state, &key, call_id) {
            Ok(pid) => Some(pid),
            Err(err) => return err,
        },
        None => None,
    };
    // A moderator's view of a webinar's questions is its stage's (§2.7).
    if pid.is_none() {
        if let Some(answer) = to_the_stage_at(
            &state,
            &key,
            reqwest::Method::GET,
            &format!("/conferences/{room_number}/questions"),
            None,
        )
        .await
        {
            return answer;
        }
    }
    match state.conference_handler.questions(&key, pid.as_deref()) {
        Ok(questions) => (
            StatusCode::OK,
            Json(serde_json::json!({ "questions": questions })),
        ),
        Err(e) => qa_error(e),
    }
}

#[derive(Debug, Deserialize)]
struct AskRequest {
    call_id: String,
    text: String,
    #[serde(default)]
    anonymous: bool,
}

/// `POST /conferences/:room/questions` — the participant on `call_id`
/// asks.
async fn ask_question_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path(room_number): Path<String>,
    Json(req): Json<AskRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = match room_key(&state, &headers, &room_number).await {
        Ok(key) => key,
        Err(err) => return err,
    };
    let pid = match participant_of_call(&state, &key, &req.call_id) {
        Ok(pid) => pid,
        Err(err) => return err,
    };
    match state
        .conference_handler
        .ask_question(&key, &pid, &req.text, req.anonymous)
    {
        Ok(view) => (StatusCode::CREATED, Json(serde_json::json!(view))),
        Err(e) => qa_error(e),
    }
}

#[derive(Debug, Deserialize)]
struct UpvoteRequest {
    call_id: String,
    on: bool,
}

/// `POST /conferences/:room/questions/:qid/upvote` — the participant on
/// `call_id` upvotes a question or takes it back.
async fn upvote_question_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path((room_number, qid)): Path<(String, String)>,
    Json(req): Json<UpvoteRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = match room_key(&state, &headers, &room_number).await {
        Ok(key) => key,
        Err(err) => return err,
    };
    let pid = match participant_of_call(&state, &key, &req.call_id) {
        Ok(pid) => pid,
        Err(err) => return err,
    };
    match state
        .conference_handler
        .upvote_question(&key, &pid, &qid, req.on)
    {
        Ok(changed) => (
            StatusCode::OK,
            Json(serde_json::json!({ "question": qid, "changed": changed })),
        ),
        Err(e) => qa_error(e),
    }
}

/// `POST /conferences/:room/questions/:qid/publish` — a host shows a
/// question to the room.
async fn publish_question_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path((room_number, qid)): Path<(String, String)>,
) -> (StatusCode, Json<serde_json::Value>) {
    moderate_handler(
        state,
        headers,
        room_number,
        qid,
        crate::conference_handler::Moderation::Publish,
    )
    .await
}

#[derive(Debug, Deserialize)]
struct AnswerRequest {
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    live: bool,
}

/// `POST /conferences/:room/questions/:qid/answer` — a host answers, in
/// text or aloud.
async fn answer_question_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path((room_number, qid)): Path<(String, String)>,
    Json(req): Json<AnswerRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    moderate_handler(
        state,
        headers,
        room_number,
        qid,
        crate::conference_handler::Moderation::Answer {
            text: req.text.as_deref(),
            live: req.live,
        },
    )
    .await
}

/// `POST /conferences/:room/questions/:qid/dismiss` — a host dismisses a
/// question.
async fn dismiss_question_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path((room_number, qid)): Path<(String, String)>,
) -> (StatusCode, Json<serde_json::Value>) {
    moderate_handler(
        state,
        headers,
        room_number,
        qid,
        crate::conference_handler::Moderation::Dismiss,
    )
    .await
}

async fn moderate_handler(
    state: Arc<ConferenceState>,
    headers: HeaderMap,
    room_number: String,
    qid: String,
    action: crate::conference_handler::Moderation<'_>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = match room_key(&state, &headers, &room_number).await {
        Ok(key) => key,
        Err(err) => return err,
    };
    // A webinar's questions are its stage's (§2.7).
    let body = match action {
        crate::conference_handler::Moderation::Answer { text, live } => {
            Some(serde_json::json!({ "text": text, "live": live }))
        }
        _ => None,
    };
    if let Some(answer) = to_the_stage_at(
        &state,
        &key,
        reqwest::Method::POST,
        &format!(
            "/conferences/{room_number}/questions/{qid}/{}",
            action.name()
        ),
        body,
    )
    .await
    {
        return answer;
    }
    match state
        .conference_handler
        .moderate_question(&key, None, &qid, action)
    {
        Ok(view) => {
            info!(room = %room_number, question = %qid, ?action, "AUDIT: a question moderated via API");
            (StatusCode::OK, Json(serde_json::json!(view)))
        }
        Err(e) => qa_error(e),
    }
}

/// The answer for a chat operation's error.
fn chat_error(e: crate::conference_handler::ChatError) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::from_u16(e.status()).unwrap_or(StatusCode::BAD_REQUEST),
        Json(serde_json::json!({ "error": e.to_string() })),
    )
}

/// The participant on `call_id` when one is named, else `None`.
fn optional_participant(
    state: &ConferenceState,
    key: &crate::rooms::RoomKey,
    call_id: Option<&str>,
) -> Result<Option<String>, (StatusCode, Json<serde_json::Value>)> {
    match call_id.filter(|c| !c.trim().is_empty()) {
        Some(call_id) => participant_of_call(state, key, call_id).map(Some),
        None => Ok(None),
    }
}

/// `GET /conferences/:room/chat[?call_id=]` — the room's last messages, as
/// the participant on that call sees them (and whether they are muted) or
/// as a host does.
async fn list_chat_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path(room_number): Path<String>,
    Query(q): Query<QuestionsQuery>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = match room_key(&state, &headers, &room_number).await {
        Ok(key) => key,
        Err(err) => return err,
    };
    let pid = match optional_participant(&state, &key, q.call_id.as_deref()) {
        Ok(pid) => pid,
        Err(err) => return err,
    };
    // A moderator's view of a webinar's chat is its stage's (§2.7).
    if pid.is_none() {
        if let Some(answer) = to_the_stage_at(
            &state,
            &key,
            reqwest::Method::GET,
            &format!("/conferences/{room_number}/chat"),
            None,
        )
        .await
        {
            return answer;
        }
    }
    match state.conference_handler.chat(&key, pid.as_deref()) {
        Ok((messages, muted)) => (
            StatusCode::OK,
            Json(serde_json::json!({ "messages": messages, "muted": muted })),
        ),
        Err(e) => chat_error(e),
    }
}

#[derive(Debug, Deserialize)]
struct PostChatRequest {
    call_id: String,
    text: String,
    #[serde(default)]
    to: fcp_core::chat::ChatAudience,
}

/// `POST /conferences/:room/chat` — the participant on `call_id` posts.
async fn post_chat_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path(room_number): Path<String>,
    Json(req): Json<PostChatRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = match room_key(&state, &headers, &room_number).await {
        Ok(key) => key,
        Err(err) => return err,
    };
    let pid = match participant_of_call(&state, &key, &req.call_id) {
        Ok(pid) => pid,
        Err(err) => return err,
    };
    match state
        .conference_handler
        .post_chat(&key, &pid, &req.text, req.to)
    {
        Ok(view) => (StatusCode::CREATED, Json(serde_json::json!(view))),
        Err(e) => chat_error(e),
    }
}

#[derive(Debug, Default, Deserialize)]
struct ChatModerationRequest {
    /// The moderator's own call, when they are in the room: the room checks
    /// they moderate and records them. Without it the caller has been
    /// checked by the API.
    #[serde(default)]
    call_id: Option<String>,
    #[serde(default)]
    on: Option<bool>,
}

/// `POST /conferences/:room/chat/:mid/delete` — a host or panelist
/// deletes a message for everyone.
async fn delete_chat_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path((room_number, mid)): Path<(String, String)>,
    body: Option<Json<ChatModerationRequest>>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = match room_key(&state, &headers, &room_number).await {
        Ok(key) => key,
        Err(err) => return err,
    };
    let req = body.map(|Json(b)| b).unwrap_or_default();
    // A webinar's chat is its stage's (§2.7).
    if let Some(answer) = to_the_stage_at(
        &state,
        &key,
        reqwest::Method::POST,
        &format!("/conferences/{room_number}/chat/{mid}/delete"),
        Some(serde_json::json!({ "call_id": req.call_id })),
    )
    .await
    {
        return answer;
    }
    let by = match optional_participant(&state, &key, req.call_id.as_deref()) {
        Ok(by) => by,
        Err(err) => return err,
    };
    match state
        .conference_handler
        .delete_chat(&key, by.as_deref(), &mid)
    {
        Ok(changed) => {
            info!(room = %room_number, message = %mid, by = ?by, "AUDIT: a chat message deleted via API");
            (
                StatusCode::OK,
                Json(serde_json::json!({ "message": mid, "changed": changed })),
            )
        }
        Err(e) => chat_error(e),
    }
}

/// `POST /conferences/:room/participants/:pid/chat-mute` — a host or
/// panelist mutes (`on`) or unmutes a participant's chat.
async fn mute_chat_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path((room_number, pid)): Path<(String, String)>,
    Json(req): Json<ChatModerationRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = match room_key(&state, &headers, &room_number).await {
        Ok(key) => key,
        Err(err) => return err,
    };
    let Some(on) = req.on else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "on is required" })),
        );
    };
    // A webinar's chat is its stage's (§2.7).
    if let Some(answer) = to_the_stage_at(
        &state,
        &key,
        reqwest::Method::POST,
        &format!("/conferences/{room_number}/participants/{pid}/chat-mute"),
        Some(serde_json::json!({ "call_id": req.call_id, "on": on })),
    )
    .await
    {
        return answer;
    }
    let by = match optional_participant(&state, &key, req.call_id.as_deref()) {
        Ok(by) => by,
        Err(err) => return err,
    };
    match state
        .conference_handler
        .mute_chat(&key, by.as_deref(), &pid, on)
    {
        Ok(changed) => {
            info!(room = %room_number, participant = %pid, on, by = ?by, "AUDIT: a participant's chat muted via API");
            (
                StatusCode::OK,
                Json(serde_json::json!({ "participant_id": pid, "on": on, "changed": changed })),
            )
        }
        Err(e) => chat_error(e),
    }
}

async fn room_stats_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path(room_number): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = match room_key(&state, &headers, &room_number).await {
        Ok(key) => key,
        Err(err) => return err,
    };
    match state.conference_handler.get_room_stats(&key) {
        Ok(stats) => (StatusCode::OK, Json(stats)),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": format!("{}", e) })),
        ),
    }
}

async fn start_participant_recording_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path((room_number, pid)): Path<(String, String)>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = match room_key(&state, &headers, &room_number).await {
        Ok(key) => key,
        Err(err) => return err,
    };
    if !is_valid_identifier(&room_number) || !is_valid_identifier(&pid) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Invalid identifier" })),
        );
    }
    match state
        .conference_handler
        .start_participant_recording(&key, &pid)
        .await
    {
        Ok(path) => (
            StatusCode::OK,
            Json(
                serde_json::json!({ "status": "recording_started", "participant": pid, "path": path }),
            ),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("{}", e) })),
        ),
    }
}

async fn stop_participant_recording_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path((room_number, pid)): Path<(String, String)>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = match room_key(&state, &headers, &room_number).await {
        Ok(key) => key,
        Err(err) => return err,
    };
    if !is_valid_identifier(&room_number) || !is_valid_identifier(&pid) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Invalid identifier" })),
        );
    }
    match state
        .conference_handler
        .stop_participant_recording(&key, &pid)
    {
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

// =========================================================================
// Control operations: lock, end, kick, mute, gain, announce
//
// Each operation is a local step plus, in a cluster, a cross-node step
// (see `crate::control`). Room-wide operations (lock, unlock, end) are
// applied here and on every other node hosting the room. Participant
// operations (kick, mute, unmute, gain) and announcements are applied
// here when the target is here, else forwarded to the hosting peers until
// one has it. The `/internal` twins apply the local step only.
// =========================================================================

/// One node's answer to a control operation: a JSON body, or a status and
/// message. `NOT_FOUND` is the signal that the target is not on this node.
type LocalResult = Result<serde_json::Value, (StatusCode, String)>;

fn local_error(status: StatusCode, err: impl std::fmt::Display) -> (StatusCode, String) {
    (status, err.to_string())
}

fn local_set_locked(state: &ConferenceState, key: &RoomKey, locked: bool) -> LocalResult {
    let room_number = key.room.as_str();
    state
        .conference_handler
        .set_room_locked(key, locked)
        .map_err(|e| local_error(StatusCode::NOT_FOUND, e))?;
    Ok(serde_json::json!({
        "status": if locked { "locked" } else { "unlocked" },
        "room": room_number,
    }))
}

async fn local_destroy(state: &ConferenceState, key: &RoomKey) -> LocalResult {
    let room_number = key.room.as_str();
    let removed = state
        .conference_handler
        .destroy_conference(key)
        .await
        .map_err(|e| local_error(StatusCode::NOT_FOUND, e))?;
    Ok(serde_json::json!({
        "status": "destroyed",
        "room_number": room_number,
        "participants_removed": removed,
    }))
}

async fn local_kick(state: &ConferenceState, key: &RoomKey, pid: &str) -> LocalResult {
    if state.bridge.get_room(key).is_err() {
        return Err(local_error(
            StatusCode::NOT_FOUND,
            "Conference room not active",
        ));
    }
    // Kick via conference handler: sends SIP BYE, aborts bridge, deallocates port
    state
        .conference_handler
        .kick_participant(pid)
        .await
        .map_err(|e| local_error(StatusCode::NOT_FOUND, e))?;
    Ok(serde_json::json!({ "status": "kicked", "participant": pid }))
}

fn local_set_muted(state: &ConferenceState, key: &RoomKey, pid: &str, muted: bool) -> LocalResult {
    let room = state
        .bridge
        .get_room(key)
        .map_err(|_| local_error(StatusCode::NOT_FOUND, "Conference room not active"))?;
    let target = if muted {
        forge_mixer::ParticipantState::Muted
    } else {
        forge_mixer::ParticipantState::Active
    };
    room.set_participant_state(pid, target)
        .map_err(|e| local_error(StatusCode::NOT_FOUND, e))?;
    Ok(serde_json::json!({
        "status": if muted { "muted" } else { "unmuted" },
        "participant": pid,
    }))
}

fn local_set_gain(state: &ConferenceState, key: &RoomKey, pid: &str, gain: f32) -> LocalResult {
    state
        .conference_handler
        .set_gain(key, pid, gain)
        .map_err(|e| local_error(StatusCode::NOT_FOUND, e))?;
    Ok(serde_json::json!({ "status": "gain_set", "participant": pid, "gain": gain }))
}

/// Play an announcement into the room on this node. The audio reaches
/// every participant on every node: the announcer is mixed like a caller
/// and so travels over the inter-node trunks. `NOT_FOUND` when the room
/// is not active here; other failures (missing file, too large) are the
/// caller's problem and are not forwarded.
async fn local_announce(state: &ConferenceState, key: &RoomKey, audio_path: &str) -> LocalResult {
    let room_number = key.room.as_str();
    if state.bridge.get_room(key).is_err() {
        return Err(local_error(
            StatusCode::NOT_FOUND,
            "Conference room not active",
        ));
    }
    // If the path doesn't contain a separator, treat it as a filename in announcements_dir
    let resolved = if audio_path.contains('/') || audio_path.contains('\\') {
        audio_path.to_string()
    } else {
        let conf = state.conference_handler.conference_config.read();
        format!("{}/{}", conf.announcements_dir, audio_path)
    };
    state
        .conference_handler
        .play_announcement(key, &resolved)
        .await
        .map_err(|e| local_error(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(serde_json::json!({ "status": "announced", "room": room_number, "audio_path": resolved }))
}

/// Reject path traversal, absolute paths, null bytes, and excessive length.
fn valid_announcement_path(audio_path: &str) -> bool {
    !(audio_path.contains("..")
        || audio_path.contains('\0')
        || audio_path.is_empty()
        || audio_path.len() > 255
        || audio_path.starts_with('/')
        || audio_path.starts_with('\\'))
}

/// The room a request names: that number in the request's tenant, which
/// the admin API and the peer nodes send in the platform tenant header.
/// Another tenant's room of the same number is another room, never this
/// one.
async fn room_key(
    state: &ConferenceState,
    headers: &HeaderMap,
    room_number: &str,
) -> Result<RoomKey, (StatusCode, Json<serde_json::Value>)> {
    Ok(RoomKey::new(
        extract_tenant_id(headers, state).await?,
        room_number,
    ))
}

/// Turn a local-only result into a response (no cluster, or an
/// `/internal` route).
/// What a peer node says is speaking in a room it hosts with us
/// (`POST /internal/conferences/{room}/speaker`, design §15.4, 5c).
///
/// A remote tile's own energy is a whole node's mix, so the node holding
/// the caller is the only one that can say who is behind it. Every node
/// feeds every claim to its own video room and runs the same rule over
/// the same candidates, so they converge on one speaker without electing
/// a leader.
#[derive(Debug, Deserialize)]
struct SpeakerClaim {
    /// The node making the claim.
    node: String,
    /// The participant holding the floor there, or absent when the node
    /// no longer has one.
    #[serde(default)]
    participant_id: Option<String>,
    #[serde(default)]
    display_name: Option<String>,
    /// The mixer's smoothed RMS for them: the same scale on every node.
    #[serde(default)]
    energy: f32,
    #[serde(default)]
    speaking: bool,
}

async fn internal_speaker_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path(room_number): Path<String>,
    Json(claim): Json<SpeakerClaim>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = match room_key(&state, &headers, &room_number).await {
        Ok(key) => key,
        Err(err) => return err,
    };
    if !is_valid_identifier(&room_number) {
        return invalid_identifier();
    }
    if claim.node.is_empty() || claim.node == state.conference_handler.node_id() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "A speaker claim must name the peer node" })),
        );
    }
    match state.conference_handler.apply_speaker_claim(
        &key,
        &claim.node,
        claim.participant_id.as_deref(),
        claim.display_name.as_deref(),
        claim.energy,
        claim.speaking,
    ) {
        Ok(applied) => (
            StatusCode::OK,
            Json(
                serde_json::json!({ "status": "applied", "room": room_number, "applied": applied }),
            ),
        ),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

/// What a peer node says is being shared in a room it hosts with us
/// (`POST /internal/conferences/{room}/content`, design §15.6, 7d): the
/// presenter there and since when. Every node keeps the earliest share
/// room-wide, so they converge on one presenter without electing a
/// leader; a claim without a participant withdraws the peer's share.
#[derive(Debug, Deserialize)]
struct ContentClaim {
    node: String,
    #[serde(default)]
    participant_id: Option<String>,
    #[serde(default)]
    display_name: Option<String>,
    /// When the share began, in milliseconds since the Unix epoch.
    #[serde(default)]
    since_ms: u64,
}

async fn internal_content_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path(room_number): Path<String>,
    Json(claim): Json<ContentClaim>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = match room_key(&state, &headers, &room_number).await {
        Ok(key) => key,
        Err(err) => return err,
    };
    if !is_valid_identifier(&room_number) {
        return invalid_identifier();
    }
    if claim.node.is_empty() || claim.node == state.conference_handler.node_id() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "A content claim must name the peer node" })),
        );
    }
    match state.conference_handler.apply_content_claim(
        &key,
        &claim.node,
        claim.participant_id.as_deref(),
        claim.display_name.as_deref(),
        claim.since_ms,
    ) {
        Ok(applied) => (
            StatusCode::OK,
            Json(
                serde_json::json!({ "status": "applied", "room": room_number, "applied": applied }),
            ),
        ),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

/// What a peer node forwarding with us says about a room
/// (`POST /internal/conferences/{room}/forwarding`,
/// `docs/FORWARDING_ROOMS.md` §2.8): its people, whose each SSRC on the
/// trunk is, and what it wants of ours.
async fn internal_forwarding_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path(room_number): Path<String>,
    Json(ann): Json<crate::trunk_forwarding::Announcement>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = match room_key(&state, &headers, &room_number).await {
        Ok(key) => key,
        Err(err) => return err,
    };
    if !is_valid_identifier(&room_number) {
        return invalid_identifier();
    }
    if ann.node.is_empty() || ann.node == state.conference_handler.node_id() {
        return (
            StatusCode::BAD_REQUEST,
            Json(
                serde_json::json!({ "error": "A forwarding announcement must name the peer node" }),
            ),
        );
    }
    match state.conference_handler.apply_forwarding(&key, &ann) {
        Ok(applied) => (
            StatusCode::OK,
            Json(
                serde_json::json!({ "status": "applied", "room": room_number, "applied": applied }),
            ),
        ),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

/// What a recording looks like over the API.
fn recording_json(recording: &fcp_core::Recording) -> serde_json::Value {
    serde_json::json!({
        "recording_id": recording.id.to_string(),
        "room": recording.room_number,
        "format": recording.format.as_str(),
        "trigger": recording.trigger.as_str(),
        "consent": recording.consent.as_str(),
        "node": recording.node_id,
        "path": recording.path,
        "file_name": recording.file_name(),
        "duration_secs": recording.duration_secs,
        "size_bytes": recording.size_bytes,
        "participants": recording.participants,
    })
}

/// Start recording the room on this node.
async fn local_recording_start(state: &ConferenceState, key: &RoomKey) -> LocalResult {
    let room_number = key.room.as_str();
    if state.bridge.get_room(key).is_err() {
        return Err(local_error(
            StatusCode::NOT_FOUND,
            "Conference room not active",
        ));
    }
    if state.conference_handler.is_room_recording(key) {
        return Err(local_error(
            StatusCode::CONFLICT,
            "The room is already being recorded",
        ));
    }
    match state
        .conference_handler
        .start_room_recording(key, fcp_core::RecordingTrigger::Api)
        .await
    {
        Ok(recording) => {
            info!(action = "start_recording", room = %room_number, recording = %recording.id, path = %recording.path, "AUDIT: recording started via API");
            let mut body = recording_json(&recording);
            body["status"] = serde_json::json!("recording_started");
            Ok(body)
        }
        Err(e) => Err(local_error(StatusCode::INTERNAL_SERVER_ERROR, e)),
    }
}

/// Stop the recording this node is making of the room.
async fn local_recording_stop(state: &ConferenceState, key: &RoomKey) -> LocalResult {
    let room_number = key.room.as_str();
    if !state.conference_handler.is_room_recording(key) {
        return Err(local_error(
            StatusCode::NOT_FOUND,
            "The room is not being recorded here",
        ));
    }
    match state.conference_handler.stop_room_recording(key).await {
        Ok(recording) => {
            info!(action = "stop_recording", room = %room_number, recording = %recording.id, "AUDIT: recording stopped via API");
            let mut body = recording_json(&recording);
            body["status"] = serde_json::json!("recording_stopped");
            Ok(body)
        }
        Err(e) => Err(local_error(StatusCode::INTERNAL_SERVER_ERROR, e)),
    }
}

fn local_response(result: LocalResult) -> (StatusCode, Json<serde_json::Value>) {
    match result {
        Ok(body) => (StatusCode::OK, Json(body)),
        Err((status, err)) => (status, Json(serde_json::json!({ "error": err }))),
    }
}

/// Room-wide operation: the local result plus every hosting peer's.
/// Succeeds when any node applied it. `body` is what to forward.
async fn room_wide(
    state: &ConferenceState,
    key: &RoomKey,
    local: LocalResult,
    method: reqwest::Method,
    path: &str,
    body: Option<serde_json::Value>,
) -> (bool, serde_json::Value) {
    let room_number = key.room.as_str();
    let tenant_id = &key.tenant;
    let Some(control) = &state.control else {
        return match local {
            Ok(body) => (true, body),
            Err((_, err)) => (false, serde_json::json!({ "error": err })),
        };
    };
    let peers = control
        .broadcast(&tenant_id.to_string(), room_number, method, path, body)
        .await;
    let mut out = match &local {
        Ok(body) => body.clone(),
        Err((_, err)) => serde_json::json!({ "error": err }),
    };
    let map = out.as_object_mut().expect("control bodies are objects");
    map.insert("node".into(), control.node_id().into());
    map.insert("hosted_here".into(), local.is_ok().into());
    let mut any_ok = local.is_ok();
    match peers {
        Ok(outcomes) => {
            any_ok |= outcomes.iter().any(|o| o.ok);
            map.insert(
                "peers".into(),
                serde_json::to_value(&outcomes).unwrap_or_default(),
            );
        }
        Err(e) => {
            warn!(room = %room_number, error = %e, "Could not reach the conference registry; peers not notified");
            map.insert("peers_error".into(), e.to_string().into());
        }
    }
    if any_ok {
        map.remove("error");
    }
    (any_ok, out)
}

/// Operation on something that lives on exactly one node: apply here,
/// and when it is not here (`NOT_FOUND`) ask the hosting peers in turn.
async fn wherever_it_is(
    state: &ConferenceState,
    key: &RoomKey,
    local: LocalResult,
    method: reqwest::Method,
    path: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let room_number = key.room.as_str();
    let tenant_id = &key.tenant;
    let (control, err) = match (&state.control, local) {
        (_, Ok(body)) => return (StatusCode::OK, Json(body)),
        (Some(control), Err((StatusCode::NOT_FOUND, err))) => (control, err),
        (_, Err((status, err))) => {
            return (status, Json(serde_json::json!({ "error": err })));
        }
    };
    let tried: Vec<PeerOutcome> = match control
        .locate(&tenant_id.to_string(), room_number, method, path, body)
        .await
    {
        Ok(tried) => tried,
        Err(e) => {
            warn!(room = %room_number, error = %e, "Could not reach the conference registry; peers not asked");
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": err, "peers_error": e.to_string() })),
            );
        }
    };
    if let Some(found) = tried.iter().find(|o| o.ok) {
        let mut out = found.body.clone();
        if let Some(map) = out.as_object_mut() {
            map.insert("node".into(), found.node.clone().into());
        }
        return (StatusCode::OK, Json(out));
    }
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({
            "error": err,
            "peers": serde_json::to_value(&tried).unwrap_or_default(),
        })),
    )
}

async fn set_locked_everywhere(
    state: &ConferenceState,
    headers: &HeaderMap,
    room_number: &str,
    locked: bool,
) -> (StatusCode, Json<serde_json::Value>) {
    let tenant_id = match extract_tenant_id(headers, state).await {
        Ok(tenant_id) => tenant_id,
        Err(err) => return err,
    };
    let key = RoomKey::new(tenant_id, room_number.to_string());
    let action = if locked { "lock" } else { "unlock" };
    let local = local_set_locked(state, &key, locked);
    let (ok, mut body) = room_wide(
        state,
        &key,
        local,
        reqwest::Method::POST,
        &format!("/conferences/{room_number}/{action}"),
        None,
    )
    .await;
    if !ok {
        return (StatusCode::NOT_FOUND, Json(body));
    }
    // The registry copy is what a node joining the room later inherits.
    if let Some(control) = &state.control {
        if let Err(e) = control
            .set_locked(&tenant_id.to_string(), room_number, locked)
            .await
        {
            warn!(room = %room_number, error = %e, "Conference registry lock flag update failed");
            if let Some(map) = body.as_object_mut() {
                map.insert("registry_error".into(), e.to_string().into());
            }
        }
    }
    info!(action, room = %room_number, "AUDIT: room {} via API", if locked { "locked" } else { "unlocked" });
    let event = if locked {
        ConferenceEvent::RoomLocked {
            room_number: room_number.to_string(),
            tenant_id,
            timestamp: SystemTime::now(),
        }
    } else {
        ConferenceEvent::RoomUnlocked {
            room_number: room_number.to_string(),
            tenant_id,
            timestamp: SystemTime::now(),
        }
    };
    publish_event(&state.event_bus, event);
    (StatusCode::OK, Json(body))
}

async fn lock_room_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path(room_number): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    if !is_valid_identifier(&room_number) {
        return invalid_identifier();
    }
    set_locked_everywhere(&state, &headers, &room_number, true).await
}

async fn unlock_room_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path(room_number): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    if !is_valid_identifier(&room_number) {
        return invalid_identifier();
    }
    set_locked_everywhere(&state, &headers, &room_number, false).await
}

async fn destroy_conference_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path(room_number): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    if !is_valid_identifier(&room_number) {
        return invalid_identifier();
    }
    let tenant_id = match extract_tenant_id(&headers, &state).await {
        Ok(tenant_id) => tenant_id,
        Err(err) => return err,
    };
    let key = RoomKey::new(tenant_id, room_number.to_string());
    let local = local_destroy(&state, &key).await;
    let local_removed = local
        .as_ref()
        .ok()
        .and_then(|b| b["participants_removed"].as_u64())
        .unwrap_or(0);
    let (ok, mut body) = room_wide(
        &state,
        &key,
        local,
        reqwest::Method::DELETE,
        &format!("/conferences/{room_number}"),
        None,
    )
    .await;
    if !ok {
        return (StatusCode::NOT_FOUND, Json(body));
    }
    // Total across the cluster, with this node's count kept separately.
    let remote_removed: u64 = body["peers"]
        .as_array()
        .map(|peers| {
            peers
                .iter()
                .filter_map(|p| p["body"]["participants_removed"].as_u64())
                .sum()
        })
        .unwrap_or(0);
    if let Some(map) = body.as_object_mut() {
        map.insert("status".into(), "destroyed".into());
        map.insert("room_number".into(), room_number.clone().into());
        map.insert("participants_removed_here".into(), local_removed.into());
        map.insert(
            "participants_removed".into(),
            (local_removed + remote_removed).into(),
        );
    }
    info!(action = "destroy", room = %room_number, removed = local_removed + remote_removed, "AUDIT: conference destroyed via API");
    (StatusCode::OK, Json(body))
}

async fn kick_participant_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path((room_number, pid)): Path<(String, String)>,
) -> (StatusCode, Json<serde_json::Value>) {
    if !is_valid_identifier(&room_number) || !is_valid_identifier(&pid) {
        return invalid_identifier();
    }
    let tenant_id = match extract_tenant_id(&headers, &state).await {
        Ok(tenant_id) => tenant_id,
        Err(err) => return err,
    };
    let key = RoomKey::new(tenant_id, room_number.to_string());
    let local = local_kick(&state, &key, &pid).await;
    let resp = wherever_it_is(
        &state,
        &key,
        local,
        reqwest::Method::POST,
        &format!("/conferences/{room_number}/kick/{pid}"),
        None,
    )
    .await;
    if resp.0 == StatusCode::OK {
        info!(action = "kick", room = %room_number, participant = %pid, node = ?resp.1["node"].as_str(), "AUDIT: participant kicked via API");
    }
    resp
}

async fn set_muted_wherever(
    state: &ConferenceState,
    headers: &HeaderMap,
    room_number: &str,
    pid: &str,
    muted: bool,
) -> (StatusCode, Json<serde_json::Value>) {
    if !is_valid_identifier(room_number) || !is_valid_identifier(pid) {
        return invalid_identifier();
    }
    let tenant_id = match extract_tenant_id(headers, state).await {
        Ok(tenant_id) => tenant_id,
        Err(err) => return err,
    };
    let key = RoomKey::new(tenant_id, room_number.to_string());
    let action = if muted { "mute" } else { "unmute" };
    let local = local_set_muted(state, &key, pid, muted);
    let resp = wherever_it_is(
        state,
        &key,
        local,
        reqwest::Method::POST,
        &format!("/conferences/{room_number}/{action}/{pid}"),
        None,
    )
    .await;
    if resp.0 == StatusCode::OK {
        info!(action, room = %room_number, participant = %pid, node = ?resp.1["node"].as_str(), "AUDIT: participant {} via API", if muted { "muted" } else { "unmuted" });
    }
    resp
}

async fn mute_participant_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path((room_number, pid)): Path<(String, String)>,
) -> (StatusCode, Json<serde_json::Value>) {
    set_muted_wherever(&state, &headers, &room_number, &pid, true).await
}

async fn unmute_participant_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path((room_number, pid)): Path<(String, String)>,
) -> (StatusCode, Json<serde_json::Value>) {
    set_muted_wherever(&state, &headers, &room_number, &pid, false).await
}

fn valid_gain(gain: f32) -> bool {
    gain.is_finite() && (0.0..=crate::audio_bridge::MAX_GAIN).contains(&gain)
}

fn invalid_gain() -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(
            serde_json::json!({ "error": format!("gain must be finite and in range [0.0, {}]", crate::audio_bridge::MAX_GAIN) }),
        ),
    )
}

fn invalid_identifier() -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({ "error": "Invalid identifier" })),
    )
}

async fn set_gain_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path((room_number, pid)): Path<(String, String)>,
    Json(payload): Json<SetGainRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    if !is_valid_identifier(&room_number) || !is_valid_identifier(&pid) {
        return invalid_identifier();
    }
    if !valid_gain(payload.gain) {
        return invalid_gain();
    }
    let tenant_id = match extract_tenant_id(&headers, &state).await {
        Ok(tenant_id) => tenant_id,
        Err(err) => return err,
    };
    let key = RoomKey::new(tenant_id, room_number.to_string());
    let local = local_set_gain(&state, &key, &pid, payload.gain);
    let resp = wherever_it_is(
        &state,
        &key,
        local,
        reqwest::Method::POST,
        &format!("/conferences/{room_number}/participants/{pid}/gain"),
        Some(serde_json::json!({ "gain": payload.gain })),
    )
    .await;
    if resp.0 == StatusCode::OK {
        info!(action = "set_gain", room = %room_number, participant = %pid, gain = payload.gain, node = ?resp.1["node"].as_str(), "AUDIT: participant gain set via API");
    }
    resp
}

async fn announce_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path(room_number): Path<String>,
    Json(payload): Json<AnnounceRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    if !is_valid_identifier(&room_number) {
        return invalid_identifier();
    }
    if !valid_announcement_path(&payload.audio_path) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Invalid audio_path" })),
        );
    }
    let tenant_id = match extract_tenant_id(&headers, &state).await {
        Ok(tenant_id) => tenant_id,
        Err(err) => return err,
    };
    let key = RoomKey::new(tenant_id, room_number.to_string());
    // Played on one hosting node; the trunks carry it to the rest. The
    // unresolved path is forwarded so the peer resolves it against its
    // own announcements_dir.
    let local = local_announce(&state, &key, &payload.audio_path).await;
    wherever_it_is(
        &state,
        &key,
        local,
        reqwest::Method::POST,
        &format!("/conferences/{room_number}/announce"),
        Some(serde_json::json!({ "audio_path": payload.audio_path })),
    )
    .await
}

// ----- video controls (design §8, §12) -----

#[derive(Debug, Deserialize)]
struct SetLayoutRequest {
    layout: String,
}

/// Body of `PUT …/video/content-layout`: `presentation` or `content_only`.
#[derive(Debug, Deserialize)]
struct SetContentLayoutRequest {
    layout: String,
}

/// The status of a video control on this node. `NOT_FOUND` when the room
/// or the participant is not here (the peers are asked next); a room here
/// with no video yet is a `CONFLICT`, an unknown layout a `BAD_REQUEST`,
/// neither forwarded.
fn video_control_status(err: &crate::video_bridge::VideoControlError) -> StatusCode {
    use crate::video_bridge::VideoControlError::*;
    match err {
        NoSuchParticipant(_) => StatusCode::NOT_FOUND,
        NoVideo => StatusCode::CONFLICT,
        UnknownLayout(_) => StatusCode::BAD_REQUEST,
        NoContent => StatusCode::CONFLICT,
        UnknownContentLayout(_) => StatusCode::BAD_REQUEST,
    }
}

fn local_video_room(
    state: &ConferenceState,
    key: &RoomKey,
) -> Result<Arc<ConferenceRoom>, (StatusCode, String)> {
    state
        .bridge
        .get_room(key)
        .map_err(|_| local_error(StatusCode::NOT_FOUND, "Conference room not active"))
}

fn local_video_control(
    state: &ConferenceState,
    key: &RoomKey,
    control: impl FnOnce(
        &ConferenceRoom,
    ) -> Result<serde_json::Value, crate::video_bridge::VideoControlError>,
) -> LocalResult {
    let room_number = key.room.as_str();
    let room = local_video_room(state, key)?;
    let mut json = control(&room).map_err(|e| (video_control_status(&e), e.to_string()))?;
    if let Some(map) = json.as_object_mut() {
        map.insert("room".into(), room_number.into());
    }
    Ok(json)
}

fn local_video_status(state: &ConferenceState, key: &RoomKey) -> LocalResult {
    state
        .conference_handler
        .get_room_video(key)
        .map_err(|_| local_error(StatusCode::NOT_FOUND, "Conference room not active"))
}

fn local_video_layout(state: &ConferenceState, key: &RoomKey, layout: &str) -> LocalResult {
    local_video_control(state, key, |room| {
        crate::video_bridge::set_layout(room, layout)
    })
}

fn local_video_pin(state: &ConferenceState, key: &RoomKey, pid: Option<&str>) -> LocalResult {
    local_video_control(state, key, |room| crate::video_bridge::pin(room, pid))
}

fn local_video_spotlight(state: &ConferenceState, key: &RoomKey, pid: Option<&str>) -> LocalResult {
    local_video_control(state, key, |room| crate::video_bridge::spotlight(room, pid))
}

fn local_participant_video(
    state: &ConferenceState,
    key: &RoomKey,
    pid: &str,
    enabled: bool,
) -> LocalResult {
    local_video_control(state, key, |room| {
        crate::video_bridge::set_participant_video(room, pid, enabled)
    })
}

fn local_content_stop(state: &ConferenceState, key: &RoomKey) -> LocalResult {
    local_video_control(state, key, crate::video_bridge::stop_content)
}

fn local_content_layout(state: &ConferenceState, key: &RoomKey, layout: &str) -> LocalResult {
    local_video_control(state, key, |room| {
        crate::video_bridge::set_content_layout(room, layout)
    })
}

/// `on` / `off` in the participant video route.
fn parse_video_state(state: &str) -> Option<bool> {
    match state {
        "on" => Some(true),
        "off" => Some(false),
        _ => None,
    }
}

fn invalid_video_state() -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({ "error": "Video state must be 'on' or 'off'" })),
    )
}

/// GET /conferences/{room}/video — the room's video and every
/// participant's, from the node hosting the room. In a room spread over
/// several nodes this is one node's view.
async fn video_status_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path(room_number): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    if !is_valid_identifier(&room_number) {
        return invalid_identifier();
    }
    let tenant_id = match extract_tenant_id(&headers, &state).await {
        Ok(tenant_id) => tenant_id,
        Err(err) => return err,
    };
    let key = RoomKey::new(tenant_id, room_number.to_string());
    let local = local_video_status(&state, &key);
    merged_video_status(&state, &key, local).await
}

/// The room's video as the whole cluster sees it (§15.4, 5c).
///
/// A room can be hosted on several nodes at once, and each composes for
/// its own callers, so one node's view is a slice. This asks every
/// hosting node and merges: the participants are the union, each stamped
/// with the node it is on, and the speaker and layout are reported as
/// agreed when the nodes agree — which they do, within a take interval,
/// because they all run the same rule over the same claims.
async fn merged_video_status(
    state: &ConferenceState,
    key: &RoomKey,
    local: LocalResult,
) -> (StatusCode, Json<serde_json::Value>) {
    let room_number = key.room.as_str();
    let tenant_id = &key.tenant;
    let Some(control) = &state.control else {
        return local_response(local);
    };
    let peers = match control
        .broadcast(
            &tenant_id.to_string(),
            room_number,
            reqwest::Method::GET,
            &format!("/conferences/{room_number}/video"),
            None,
        )
        .await
    {
        Ok(peers) => peers,
        Err(e) => {
            warn!(room = %room_number, error = %e, "Could not reach the registry; video status is this node's only");
            return local_response(local);
        }
    };

    // Every node's view, this one first when it hosts the room.
    let mut views: Vec<(String, serde_json::Value)> = Vec::new();
    if let Ok(body) = &local {
        views.push((state.conference_handler.node_id().to_string(), body.clone()));
    }
    for peer in peers.iter().filter(|p| p.ok) {
        views.push((peer.node.clone(), peer.body.clone()));
    }
    if views.is_empty() {
        return local_response(local);
    }

    let mut participants: Vec<serde_json::Value> = Vec::new();
    let mut trunks: Vec<serde_json::Value> = Vec::new();
    let mut speakers: Vec<String> = Vec::new();
    let mut layouts: Vec<String> = Vec::new();
    let mut count = 0u64;
    // The share is the room's too (§15.6, 7d): the node whose own caller
    // holds the floor names the presenter; every other node sees them
    // through a trunk.
    let mut content: Option<serde_json::Value> = None;
    for (node, view) in &views {
        if let Some(list) = view["participants"].as_array() {
            for p in list {
                let mut p = p.clone();
                if let Some(map) = p.as_object_mut() {
                    // Which node a caller is on decides where a control
                    // request for them has to go.
                    map.insert("node_id".into(), node.clone().into());
                }
                participants.push(p);
            }
        }
        if let Some(list) = view["trunks"].as_array() {
            trunks.extend(list.iter().cloned());
        }
        count += view["participant_count"].as_u64().unwrap_or(0);
        if let Some(s) = view["video"]["active_speaker"].as_str() {
            speakers.push(s.to_string());
        }
        if let Some(l) = view["video"]["layout"].as_str() {
            layouts.push(l.to_string());
        }
        if content.is_none() && view["video"]["content"]["remote"] == serde_json::json!(false) {
            let mut c = view["video"]["content"].clone();
            if let Some(map) = c.as_object_mut() {
                map.insert("node_id".into(), node.clone().into());
            }
            content = Some(c);
        }
    }
    let agreed = |values: &[String]| -> (Option<String>, bool) {
        match values.first() {
            Some(first) => (Some(first.clone()), values.iter().all(|v| v == first)),
            None => (None, true),
        }
    };
    let (speaker, speaker_agreed) = agreed(&speakers);
    let (layout, layout_agreed) = agreed(&layouts);

    let mut body = match &local {
        Ok(body) => body.clone(),
        // The room is not here: the first hosting node's view is the
        // shape to build on.
        Err(_) => views[0].1.clone(),
    };
    if let Some(map) = body.as_object_mut() {
        map.insert("participants".into(), participants.into());
        map.insert("participant_count".into(), count.into());
        map.insert("trunks".into(), trunks.into());
        map.insert(
            "nodes".into(),
            views
                .iter()
                .map(|(node, view)| {
                    serde_json::json!({
                        "node_id": node,
                        "participant_count": view["participant_count"],
                        "active_speaker": view["video"]["active_speaker"],
                        "active_speaker_via": view["video"]["active_speaker_via"],
                        "layout": view["video"]["layout"],
                        "content": view["video"]["content"]["participant_id"],
                    })
                })
                .collect::<Vec<_>>()
                .into(),
        );
        map.insert("hosted_here".into(), local.is_ok().into());
        // The speaker and the layout are the room's, not a node's, so
        // they are lifted out of the per-node video block; the share,
        // named by the presenter's own node, replaces this node's view
        // of it (a trunk's tile) in the video block.
        map.insert("active_speaker".into(), speaker.into());
        map.insert("layout".into(), layout.into());
        if let Some(video) = map.get_mut("video").and_then(|v| v.as_object_mut()) {
            video.insert("content".into(), content.unwrap_or(serde_json::Value::Null));
        }
        // False only while the nodes are still converging.
        map.insert(
            "agreed".into(),
            serde_json::json!(speaker_agreed && layout_agreed),
        );
    }
    (StatusCode::OK, Json(body))
}

/// PUT /conferences/{room}/video/layout `{"layout": "grid" | "active_speaker" | "spotlight" | "pip"}`
/// — room-wide: every node hosting the room composes with it.
async fn video_layout_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path(room_number): Path<String>,
    Json(payload): Json<SetLayoutRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    if !is_valid_identifier(&room_number) {
        return invalid_identifier();
    }
    if forge_video::layout::Layout::parse(&payload.layout).is_none() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": format!("Unknown layout '{}' (one of grid, active_speaker, spotlight, pip)", payload.layout)
            })),
        );
    }
    let tenant_id = match extract_tenant_id(&headers, &state).await {
        Ok(tenant_id) => tenant_id,
        Err(err) => return err,
    };
    let key = RoomKey::new(tenant_id, room_number.to_string());
    let local = local_video_layout(&state, &key, &payload.layout);
    let local_status = local.as_ref().err().map(|(s, _)| *s);
    let (ok, body) = room_wide(
        &state,
        &key,
        local,
        reqwest::Method::PUT,
        &format!("/conferences/{room_number}/video/layout"),
        Some(serde_json::json!({ "layout": payload.layout })),
    )
    .await;
    if !ok {
        // Not applied anywhere: the room is not here (and no peer took
        // it), or it is here without video.
        let status = local_status.unwrap_or(StatusCode::NOT_FOUND);
        return (status, Json(body));
    }
    info!(action = "video_layout", room = %room_number, layout = %payload.layout, node = ?body["node"].as_str(), "AUDIT: video layout set via API");
    (StatusCode::OK, Json(body))
}

async fn video_pin_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path((room_number, pid)): Path<(String, String)>,
) -> (StatusCode, Json<serde_json::Value>) {
    if !is_valid_identifier(&room_number) || !is_valid_identifier(&pid) {
        return invalid_identifier();
    }
    let tenant_id = match extract_tenant_id(&headers, &state).await {
        Ok(tenant_id) => tenant_id,
        Err(err) => return err,
    };
    let key = RoomKey::new(tenant_id, room_number.to_string());
    let local = local_video_pin(&state, &key, Some(&pid));
    let resp = wherever_it_is(
        &state,
        &key,
        local,
        reqwest::Method::POST,
        &format!("/conferences/{room_number}/video/pin/{pid}"),
        None,
    )
    .await;
    if resp.0 == StatusCode::OK {
        info!(action = "video_pin", room = %room_number, participant = %pid, node = ?resp.1["node"].as_str(), "AUDIT: participant pinned via API");
    }
    resp
}

/// DELETE /conferences/{room}/video/pin — room-wide, since the pin may be
/// held on whichever node the pinned participant was on.
async fn video_unpin_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path(room_number): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    clear_video_selection(&state, &headers, &room_number, "pin").await
}

async fn video_spotlight_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path((room_number, pid)): Path<(String, String)>,
) -> (StatusCode, Json<serde_json::Value>) {
    if !is_valid_identifier(&room_number) || !is_valid_identifier(&pid) {
        return invalid_identifier();
    }
    let tenant_id = match extract_tenant_id(&headers, &state).await {
        Ok(tenant_id) => tenant_id,
        Err(err) => return err,
    };
    let key = RoomKey::new(tenant_id, room_number.to_string());
    let local = local_video_spotlight(&state, &key, Some(&pid));
    let resp = wherever_it_is(
        &state,
        &key,
        local,
        reqwest::Method::POST,
        &format!("/conferences/{room_number}/video/spotlight/{pid}"),
        None,
    )
    .await;
    if resp.0 == StatusCode::OK {
        info!(action = "video_spotlight", room = %room_number, participant = %pid, node = ?resp.1["node"].as_str(), "AUDIT: participant spotlighted via API");
    }
    resp
}

async fn video_unspotlight_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path(room_number): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    clear_video_selection(&state, &headers, &room_number, "spotlight").await
}

/// Clear the pin or the spotlight on every node hosting the room.
async fn clear_video_selection(
    state: &ConferenceState,
    headers: &HeaderMap,
    room_number: &str,
    which: &str,
) -> (StatusCode, Json<serde_json::Value>) {
    if !is_valid_identifier(room_number) {
        return invalid_identifier();
    }
    let tenant_id = match extract_tenant_id(headers, state).await {
        Ok(tenant_id) => tenant_id,
        Err(err) => return err,
    };
    let key = RoomKey::new(tenant_id, room_number.to_string());
    let local = match which {
        "pin" => local_video_pin(state, &key, None),
        _ => local_video_spotlight(state, &key, None),
    };
    let local_status = local.as_ref().err().map(|(s, _)| *s);
    let (ok, body) = room_wide(
        state,
        &key,
        local,
        reqwest::Method::DELETE,
        &format!("/conferences/{room_number}/video/{which}"),
        None,
    )
    .await;
    if !ok {
        return (local_status.unwrap_or(StatusCode::NOT_FOUND), Json(body));
    }
    info!(action = "video_clear", room = %room_number, which, node = ?body["node"].as_str(), "AUDIT: video {} cleared via API", which);
    (StatusCode::OK, Json(body))
}

/// POST /conferences/{room}/participants/{pid}/video/{on|off} — a host
/// hiding or showing one participant's video.
async fn participant_video_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path((room_number, pid, video_state)): Path<(String, String, String)>,
) -> (StatusCode, Json<serde_json::Value>) {
    if !is_valid_identifier(&room_number) || !is_valid_identifier(&pid) {
        return invalid_identifier();
    }
    let Some(enabled) = parse_video_state(&video_state) else {
        return invalid_video_state();
    };
    let tenant_id = match extract_tenant_id(&headers, &state).await {
        Ok(tenant_id) => tenant_id,
        Err(err) => return err,
    };
    let key = RoomKey::new(tenant_id, room_number.to_string());
    let local = local_participant_video(&state, &key, &pid, enabled);
    let resp = wherever_it_is(
        &state,
        &key,
        local,
        reqwest::Method::POST,
        &format!("/conferences/{room_number}/participants/{pid}/video/{video_state}"),
        None,
    )
    .await;
    if resp.0 == StatusCode::OK {
        info!(action = "participant_video", room = %room_number, participant = %pid, enabled, node = ?resp.1["node"].as_str(), "AUDIT: participant video {} via API", if enabled { "enabled" } else { "disabled" });
    }
    resp
}

/// POST /conferences/{room}/video/content/stop — a host ends the share
/// under way (§15.6). Room-wide: whichever hosting node has the
/// presenter applies it, the others answer that nobody is sharing there.
async fn content_stop_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path(room_number): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    if !is_valid_identifier(&room_number) {
        return invalid_identifier();
    }
    let tenant_id = match extract_tenant_id(&headers, &state).await {
        Ok(tenant_id) => tenant_id,
        Err(err) => return err,
    };
    let key = RoomKey::new(tenant_id, room_number.to_string());
    let local = local_content_stop(&state, &key);
    let local_status = local.as_ref().err().map(|(s, _)| *s);
    let (ok, body) = room_wide(
        &state,
        &key,
        local,
        reqwest::Method::POST,
        &format!("/conferences/{room_number}/video/content/stop"),
        None,
    )
    .await;
    if !ok {
        let status = local_status.unwrap_or(StatusCode::NOT_FOUND);
        return (status, Json(body));
    }
    info!(action = "content_stop", room = %room_number, participant = ?body["participant"].as_str(), node = ?body["node"].as_str(), "AUDIT: screen share stopped via API");
    (StatusCode::OK, Json(body))
}

/// PUT /conferences/{room}/video/content-layout `{"layout": "presentation" | "content_only"}`
/// — room-wide, like the layout.
async fn content_layout_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path(room_number): Path<String>,
    Json(payload): Json<SetContentLayoutRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    if !is_valid_identifier(&room_number) {
        return invalid_identifier();
    }
    if forge_conference::video::ContentLayout::parse(&payload.layout).is_none() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": format!("Unknown content layout '{}' (one of presentation, content_only)", payload.layout)
            })),
        );
    }
    let tenant_id = match extract_tenant_id(&headers, &state).await {
        Ok(tenant_id) => tenant_id,
        Err(err) => return err,
    };
    let key = RoomKey::new(tenant_id, room_number.to_string());
    let local = local_content_layout(&state, &key, &payload.layout);
    let local_status = local.as_ref().err().map(|(s, _)| *s);
    let (ok, body) = room_wide(
        &state,
        &key,
        local,
        reqwest::Method::PUT,
        &format!("/conferences/{room_number}/video/content-layout"),
        Some(serde_json::json!({ "layout": payload.layout })),
    )
    .await;
    if !ok {
        let status = local_status.unwrap_or(StatusCode::NOT_FOUND);
        return (status, Json(body));
    }
    info!(action = "content_layout", room = %room_number, layout = %payload.layout, node = ?body["node"].as_str(), "AUDIT: content layout set via API");
    (StatusCode::OK, Json(body))
}

// ----- /internal twins: the local step only, on behalf of a peer node -----

fn forwarded_from(headers: &HeaderMap) -> String {
    headers
        .get(NODE_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("?")
        .to_string()
}

async fn internal_destroy_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path(room_number): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = match room_key(&state, &headers, &room_number).await {
        Ok(key) => key,
        Err(err) => return err,
    };
    if !is_valid_identifier(&room_number) {
        return invalid_identifier();
    }
    let resp = local_response(local_destroy(&state, &key).await);
    if resp.0 == StatusCode::OK {
        info!(action = "destroy", room = %room_number, from = %forwarded_from(&headers), "AUDIT: conference destroyed on behalf of peer node");
    }
    resp
}

async fn internal_lock_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path(room_number): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = match room_key(&state, &headers, &room_number).await {
        Ok(key) => key,
        Err(err) => return err,
    };
    if !is_valid_identifier(&room_number) {
        return invalid_identifier();
    }
    let resp = local_response(local_set_locked(&state, &key, true));
    if resp.0 == StatusCode::OK {
        info!(action = "lock", room = %room_number, from = %forwarded_from(&headers), "AUDIT: room locked on behalf of peer node");
    }
    resp
}

async fn internal_unlock_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path(room_number): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = match room_key(&state, &headers, &room_number).await {
        Ok(key) => key,
        Err(err) => return err,
    };
    if !is_valid_identifier(&room_number) {
        return invalid_identifier();
    }
    let resp = local_response(local_set_locked(&state, &key, false));
    if resp.0 == StatusCode::OK {
        info!(action = "unlock", room = %room_number, from = %forwarded_from(&headers), "AUDIT: room unlocked on behalf of peer node");
    }
    resp
}

async fn internal_kick_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path((room_number, pid)): Path<(String, String)>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = match room_key(&state, &headers, &room_number).await {
        Ok(key) => key,
        Err(err) => return err,
    };
    if !is_valid_identifier(&room_number) || !is_valid_identifier(&pid) {
        return invalid_identifier();
    }
    let resp = local_response(local_kick(&state, &key, &pid).await);
    if resp.0 == StatusCode::OK {
        info!(action = "kick", room = %room_number, participant = %pid, from = %forwarded_from(&headers), "AUDIT: participant kicked on behalf of peer node");
    }
    resp
}

async fn internal_mute_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path((room_number, pid)): Path<(String, String)>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = match room_key(&state, &headers, &room_number).await {
        Ok(key) => key,
        Err(err) => return err,
    };
    if !is_valid_identifier(&room_number) || !is_valid_identifier(&pid) {
        return invalid_identifier();
    }
    let resp = local_response(local_set_muted(&state, &key, &pid, true));
    if resp.0 == StatusCode::OK {
        info!(action = "mute", room = %room_number, participant = %pid, from = %forwarded_from(&headers), "AUDIT: participant muted on behalf of peer node");
    }
    resp
}

async fn internal_unmute_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path((room_number, pid)): Path<(String, String)>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = match room_key(&state, &headers, &room_number).await {
        Ok(key) => key,
        Err(err) => return err,
    };
    if !is_valid_identifier(&room_number) || !is_valid_identifier(&pid) {
        return invalid_identifier();
    }
    let resp = local_response(local_set_muted(&state, &key, &pid, false));
    if resp.0 == StatusCode::OK {
        info!(action = "unmute", room = %room_number, participant = %pid, from = %forwarded_from(&headers), "AUDIT: participant unmuted on behalf of peer node");
    }
    resp
}

async fn internal_gain_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path((room_number, pid)): Path<(String, String)>,
    Json(payload): Json<SetGainRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = match room_key(&state, &headers, &room_number).await {
        Ok(key) => key,
        Err(err) => return err,
    };
    if !is_valid_identifier(&room_number) || !is_valid_identifier(&pid) {
        return invalid_identifier();
    }
    if !valid_gain(payload.gain) {
        return invalid_gain();
    }
    let resp = local_response(local_set_gain(&state, &key, &pid, payload.gain));
    if resp.0 == StatusCode::OK {
        info!(action = "set_gain", room = %room_number, participant = %pid, gain = payload.gain, from = %forwarded_from(&headers), "AUDIT: participant gain set on behalf of peer node");
    }
    resp
}

async fn internal_announce_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path(room_number): Path<String>,
    Json(payload): Json<AnnounceRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = match room_key(&state, &headers, &room_number).await {
        Ok(key) => key,
        Err(err) => return err,
    };
    if !is_valid_identifier(&room_number) {
        return invalid_identifier();
    }
    if !valid_announcement_path(&payload.audio_path) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Invalid audio_path" })),
        );
    }
    let resp = local_response(local_announce(&state, &key, &payload.audio_path).await);
    if resp.0 == StatusCode::OK {
        info!(action = "announce", room = %room_number, from = %forwarded_from(&headers), "Announcement played on behalf of peer node");
    }
    resp
}

/// POST /internal/conferences/{room}/recording/start — record here, for
/// a peer that has the request but not the room.
async fn internal_recording_start_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path(room_number): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = match room_key(&state, &headers, &room_number).await {
        Ok(key) => key,
        Err(err) => return err,
    };
    if !is_valid_identifier(&room_number) {
        return invalid_identifier();
    }
    local_response(local_recording_start(&state, &key).await)
}

/// POST /internal/conferences/{room}/recording/stop — stop the recording
/// this node is making, for a peer.
async fn internal_recording_stop_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path(room_number): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = match room_key(&state, &headers, &room_number).await {
        Ok(key) => key,
        Err(err) => return err,
    };
    if !is_valid_identifier(&room_number) {
        return invalid_identifier();
    }
    local_response(local_recording_stop(&state, &key).await)
}

async fn internal_video_status_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path(room_number): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = match room_key(&state, &headers, &room_number).await {
        Ok(key) => key,
        Err(err) => return err,
    };
    if !is_valid_identifier(&room_number) {
        return invalid_identifier();
    }
    local_response(local_video_status(&state, &key))
}

async fn internal_video_layout_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path(room_number): Path<String>,
    Json(payload): Json<SetLayoutRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = match room_key(&state, &headers, &room_number).await {
        Ok(key) => key,
        Err(err) => return err,
    };
    if !is_valid_identifier(&room_number) {
        return invalid_identifier();
    }
    let resp = local_response(local_video_layout(&state, &key, &payload.layout));
    if resp.0 == StatusCode::OK {
        info!(action = "video_layout", room = %room_number, layout = %payload.layout, from = %forwarded_from(&headers), "AUDIT: video layout set on behalf of peer node");
    }
    resp
}

async fn internal_video_pin_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path((room_number, pid)): Path<(String, String)>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = match room_key(&state, &headers, &room_number).await {
        Ok(key) => key,
        Err(err) => return err,
    };
    if !is_valid_identifier(&room_number) || !is_valid_identifier(&pid) {
        return invalid_identifier();
    }
    let resp = local_response(local_video_pin(&state, &key, Some(&pid)));
    if resp.0 == StatusCode::OK {
        info!(action = "video_pin", room = %room_number, participant = %pid, from = %forwarded_from(&headers), "AUDIT: participant pinned on behalf of peer node");
    }
    resp
}

async fn internal_video_unpin_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path(room_number): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = match room_key(&state, &headers, &room_number).await {
        Ok(key) => key,
        Err(err) => return err,
    };
    if !is_valid_identifier(&room_number) {
        return invalid_identifier();
    }
    let resp = local_response(local_video_pin(&state, &key, None));
    if resp.0 == StatusCode::OK {
        info!(action = "video_clear", room = %room_number, which = "pin", from = %forwarded_from(&headers), "AUDIT: video pin cleared on behalf of peer node");
    }
    resp
}

async fn internal_video_spotlight_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path((room_number, pid)): Path<(String, String)>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = match room_key(&state, &headers, &room_number).await {
        Ok(key) => key,
        Err(err) => return err,
    };
    if !is_valid_identifier(&room_number) || !is_valid_identifier(&pid) {
        return invalid_identifier();
    }
    let resp = local_response(local_video_spotlight(&state, &key, Some(&pid)));
    if resp.0 == StatusCode::OK {
        info!(action = "video_spotlight", room = %room_number, participant = %pid, from = %forwarded_from(&headers), "AUDIT: participant spotlighted on behalf of peer node");
    }
    resp
}

async fn internal_video_unspotlight_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path(room_number): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = match room_key(&state, &headers, &room_number).await {
        Ok(key) => key,
        Err(err) => return err,
    };
    if !is_valid_identifier(&room_number) {
        return invalid_identifier();
    }
    let resp = local_response(local_video_spotlight(&state, &key, None));
    if resp.0 == StatusCode::OK {
        info!(action = "video_clear", room = %room_number, which = "spotlight", from = %forwarded_from(&headers), "AUDIT: video spotlight cleared on behalf of peer node");
    }
    resp
}

async fn internal_content_stop_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path(room_number): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = match room_key(&state, &headers, &room_number).await {
        Ok(key) => key,
        Err(err) => return err,
    };
    if !is_valid_identifier(&room_number) {
        return invalid_identifier();
    }
    let resp = local_response(local_content_stop(&state, &key));
    if resp.0 == StatusCode::OK {
        info!(action = "content_stop", room = %room_number, from = %forwarded_from(&headers), "AUDIT: screen share stopped on behalf of peer node");
    }
    resp
}

async fn internal_content_layout_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path(room_number): Path<String>,
    Json(payload): Json<SetContentLayoutRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = match room_key(&state, &headers, &room_number).await {
        Ok(key) => key,
        Err(err) => return err,
    };
    if !is_valid_identifier(&room_number) {
        return invalid_identifier();
    }
    let resp = local_response(local_content_layout(&state, &key, &payload.layout));
    if resp.0 == StatusCode::OK {
        info!(action = "content_layout", room = %room_number, layout = %payload.layout, from = %forwarded_from(&headers), "AUDIT: content layout set on behalf of peer node");
    }
    resp
}

async fn internal_participant_video_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path((room_number, pid, video_state)): Path<(String, String, String)>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = match room_key(&state, &headers, &room_number).await {
        Ok(key) => key,
        Err(err) => return err,
    };
    if !is_valid_identifier(&room_number) || !is_valid_identifier(&pid) {
        return invalid_identifier();
    }
    let Some(enabled) = parse_video_state(&video_state) else {
        return invalid_video_state();
    };
    let resp = local_response(local_participant_video(&state, &key, &pid, enabled));
    if resp.0 == StatusCode::OK {
        info!(action = "participant_video", room = %room_number, participant = %pid, enabled, from = %forwarded_from(&headers), "AUDIT: participant video set on behalf of peer node");
    }
    resp
}

// =========================================================================
// Admin: Admit Waiting Participants
// =========================================================================

async fn admit_participant_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path((room_number, pid)): Path<(String, String)>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = match room_key(&state, &headers, &room_number).await {
        Ok(key) => key,
        Err(err) => return err,
    };
    if !is_valid_identifier(&room_number) || !is_valid_identifier(&pid) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Invalid identifier" })),
        );
    }
    match state.conference_handler.admit_waiting(&key, &pid) {
        Ok(()) => {
            info!(action = "admit", room = %room_number, participant = %pid, "AUDIT: waiting participant admitted via API");
            (
                StatusCode::OK,
                Json(serde_json::json!({ "status": "admitted", "participant": pid })),
            )
        }
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": format!("{}", e) })),
        ),
    }
}

async fn admit_all_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path(room_number): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = match room_key(&state, &headers, &room_number).await {
        Ok(key) => key,
        Err(err) => return err,
    };
    match state.conference_handler.admit_all_waiting(&key) {
        Ok(count) => {
            info!(action = "admit_all", room = %room_number, count = count, "AUDIT: all waiting participants admitted via API");
            (
                StatusCode::OK,
                Json(serde_json::json!({ "status": "admitted_all", "count": count })),
            )
        }
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
    headers: HeaderMap,
    Path(room_number): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = match room_key(&state, &headers, &room_number).await {
        Ok(key) => key,
        Err(err) => return err,
    };
    match state.conference_handler.attach_ai(&key).await {
        Ok(()) => {
            info!(action = "attach_ai", room = %room_number, "AUDIT: AI participant attached via API");
            (
                StatusCode::OK,
                Json(serde_json::json!({ "status": "ai_attached", "room": room_number })),
            )
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("{}", e) })),
        ),
    }
}

async fn detach_ai_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path(room_number): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = match room_key(&state, &headers, &room_number).await {
        Ok(key) => key,
        Err(err) => return err,
    };
    match state.conference_handler.detach_ai(&key).await {
        Ok(()) => {
            info!(action = "detach_ai", room = %room_number, "AUDIT: AI participant detached via API");
            (
                StatusCode::OK,
                Json(serde_json::json!({ "status": "ai_detached", "room": room_number })),
            )
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("{}", e) })),
        ),
    }
}

async fn ai_status_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path(room_number): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = match room_key(&state, &headers, &room_number).await {
        Ok(key) => key,
        Err(err) => return err,
    };
    match state.conference_handler.ai_status(&key) {
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
    Query(pagination): Query<PaginationParams>,
) -> (StatusCode, Json<serde_json::Value>) {
    let recordings_dir = state.conference_handler.recordings_dir();

    let mut read_dir = match tokio::fs::read_dir(&recordings_dir).await {
        Ok(rd) => rd,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Cannot read recordings dir: {}", e) })),
            );
        }
    };

    let mut all_recordings = Vec::new();
    while let Ok(Some(entry)) = read_dir.next_entry().await {
        let path = entry.path();
        // The containers a room records to (§11, §15.9): WAV for
        // audio-only meetings, MP4 or WebM when the room has video.
        if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("wav") | Some("webm") | Some("mp4")
        ) {
            let filename = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let meta = tokio::fs::metadata(&path).await.ok();
            let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
            let modified = meta
                .as_ref()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            all_recordings.push(serde_json::json!({
                "filename": filename,
                "size_bytes": size,
                "modified_at": modified,
            }));
        }
    }

    let total = all_recordings.len();
    let page: Vec<serde_json::Value> = all_recordings
        .into_iter()
        .skip(pagination.offset)
        .take(pagination.limit)
        .collect();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "recordings": page,
            "total": total,
            "offset": pagination.offset,
            "limit": pagination.limit,
        })),
    )
}

/// Resolve a recording filename to a safe, canonicalized path within recordings_dir.
/// Returns None if the path escapes the recordings directory.
fn safe_recording_path(recordings_dir: &str, filename: &str) -> Option<std::path::PathBuf> {
    // Reject filenames with path separators, parent traversals, or null bytes
    if filename.contains('/')
        || filename.contains('\\')
        || filename.contains("..")
        || filename.contains('\0')
        || filename.is_empty()
    {
        return None;
    }

    let base = std::path::Path::new(recordings_dir);
    let candidate = base.join(filename);

    // Canonicalize both paths (resolves symlinks) and verify containment
    let base_canon = base.canonicalize().ok()?;
    let candidate_canon = candidate.canonicalize().ok()?;

    if candidate_canon.starts_with(&base_canon) {
        Some(candidate_canon)
    } else {
        None
    }
}

/// Which node holds the file. A recording is a row first, and the row
/// says which node's disk the file is on, so the admin API can ask any
/// node and name the one it wants.
#[derive(Deserialize)]
struct NodeParam {
    node: Option<String>,
}

/// GET /recordings/{filename} — this node's file, or the named node's,
/// streamed through.
async fn download_recording_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path(filename): Path<String>,
    Query(NodeParam { node }): Query<NodeParam>,
) -> axum::response::Response {
    match peer_for(&state, node.as_deref()) {
        PeerRoute::Local => {}
        PeerRoute::Unknown(node) => {
            return json_response(
                StatusCode::SERVICE_UNAVAILABLE,
                serde_json::json!({ "error": format!("Node {node} is not in cluster membership") }),
            );
        }
        PeerRoute::Peer(node) => return stream_from_peer(&state, &headers, &node, &filename).await,
    }
    local_download(&state, &filename).await
}

/// GET /internal/recordings/{filename} — what a peer asks this node for.
async fn internal_download_recording_handler(
    State(state): State<Arc<ConferenceState>>,
    Path(filename): Path<String>,
) -> axum::response::Response {
    local_download(&state, &filename).await
}

fn json_response(status: StatusCode, body: serde_json::Value) -> axum::response::Response {
    (status, Json(body)).into_response()
}

/// Where a `?node=` parameter points.
enum PeerRoute {
    Local,
    Peer(String),
    Unknown(String),
}

/// What may go into a peer's URL path: a bare file name, so nothing a
/// caller sends can change which request the peer sees. The peer checks
/// the name against its own directory again ([`safe_recording_path`]).
fn forwardable_file_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && !name.contains("..")
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

fn peer_for(state: &ConferenceState, node: Option<&str>) -> PeerRoute {
    let Some(node) = node.filter(|n| !n.is_empty()) else {
        return PeerRoute::Local;
    };
    if node == state.conference_handler.node_id() {
        return PeerRoute::Local;
    }
    match &state.control {
        Some(control) if control.member(node).is_some() => PeerRoute::Peer(node.to_string()),
        _ => PeerRoute::Unknown(node.to_string()),
    }
}

/// Stream a peer's copy of the file back to our caller.
async fn stream_from_peer(
    state: &ConferenceState,
    headers: &HeaderMap,
    node: &str,
    filename: &str,
) -> axum::response::Response {
    let Some(control) = &state.control else {
        return json_response(
            StatusCode::SERVICE_UNAVAILABLE,
            serde_json::json!({ "error": "This node is not clustered" }),
        );
    };
    if !forwardable_file_name(filename) {
        return json_response(
            StatusCode::BAD_REQUEST,
            serde_json::json!({ "error": "Invalid filename" }),
        );
    }
    let tenant = headers
        .get(PLATFORM_TENANT_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let path = format!("/recordings/{filename}");
    match control.stream_from_node(node, &tenant, &path).await {
        Ok(resp) => {
            let status =
                StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            let content_type = resp
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("application/octet-stream")
                .to_string();
            let stream = resp.bytes_stream();
            (
                status,
                [("content-type", content_type)],
                Body::from_stream(stream),
            )
                .into_response()
        }
        Err(e) => {
            warn!(node = %node, file = %filename, error = %e, "Could not fetch a recording from its node");
            json_response(
                StatusCode::BAD_GATEWAY,
                serde_json::json!({ "error": format!("Node {node}: {e}") }),
            )
        }
    }
}

/// A webinar's stream by its sitting (`docs/WEBINARS.md` §2.8): the
/// directory under `[conference.hls] dir`, when `name` names one.
fn stream_dir(state: &ConferenceState, name: &str) -> Option<std::path::PathBuf> {
    if !crate::recording::hls::is_sitting_name(name) {
        return None;
    }
    let dir = state.conference_handler.hls_dir().join(name);
    dir.is_dir().then_some(dir)
}

/// A stream downloaded: its init segment and its segments, in order, one
/// fragmented MP4. A stream still being written comes down as far as it
/// has got.
async fn stream_download(dir: std::path::PathBuf) -> axum::response::Response {
    use futures::{StreamExt as _, TryStreamExt as _};
    let files = match tokio::task::spawn_blocking(move || crate::recording::hls::stream_files(&dir))
        .await
    {
        Ok(Ok(files)) if !files.is_empty() => files,
        Ok(Ok(_)) => {
            return json_response(
                StatusCode::NOT_FOUND,
                serde_json::json!({ "error": "The stream has no init segment yet" }),
            )
        }
        Ok(Err(e)) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                serde_json::json!({ "error": format!("Failed to read the stream: {e}") }),
            )
        }
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                serde_json::json!({ "error": e.to_string() }),
            )
        }
    };
    let body = futures::stream::iter(files)
        .then(|path| async move { tokio::fs::File::open(path).await })
        .map_ok(tokio_util::io::ReaderStream::new)
        .try_flatten();
    (
        StatusCode::OK,
        [("content-type", "video/mp4")],
        Body::from_stream(body),
    )
        .into_response()
}

async fn local_download(state: &ConferenceState, filename: &str) -> axum::response::Response {
    if let Some(dir) = stream_dir(state, filename) {
        return stream_download(dir).await;
    }
    let recordings_dir = state.conference_handler.recordings_dir();
    let Some(path) = safe_recording_path(&recordings_dir, filename) else {
        return json_response(
            StatusCode::BAD_REQUEST,
            serde_json::json!({ "error": "Invalid filename" }),
        );
    };

    if !path.exists() {
        return json_response(
            StatusCode::NOT_FOUND,
            serde_json::json!({ "error": "Recording not found" }),
        );
    }

    let file = match tokio::fs::File::open(&path).await {
        Ok(f) => f,
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                serde_json::json!({ "error": format!("Failed to open file: {e}") }),
            );
        }
    };

    let content_type = match path.extension().and_then(|e| e.to_str()) {
        Some("webm") => "video/webm",
        Some("mp4") => "video/mp4",
        _ => "audio/wav",
    };
    let stream = tokio_util::io::ReaderStream::new(file);
    (
        StatusCode::OK,
        [("content-type", content_type)],
        Body::from_stream(stream),
    )
        .into_response()
}

/// `GET /hls/{sitting}/{file}` — a file of a webinar's HLS stream on this
/// node: its playlist (`index.m3u8`, never cached), its init segment or a
/// segment (`seg-<n>.m4s`, never changed once written). Any other name is
/// refused.
async fn hls_file_handler(
    State(state): State<Arc<ConferenceState>>,
    Path((sitting, file)): Path<(String, String)>,
) -> axum::response::Response {
    use crate::recording::hls;
    let sitting_ok = !sitting.is_empty()
        && sitting.len() <= 64
        && sitting
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-');
    if !sitting_ok || !hls::is_stream_file(&file) {
        return json_response(
            StatusCode::BAD_REQUEST,
            serde_json::json!({ "error": "Invalid stream file" }),
        );
    }
    let path = state
        .conference_handler
        .hls_dir()
        .join(&sitting)
        .join(&file);
    let bytes = match tokio::fs::read(&path).await {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return json_response(
                StatusCode::NOT_FOUND,
                serde_json::json!({ "error": "No such stream file" }),
            );
        }
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                serde_json::json!({ "error": format!("Failed to read the stream: {e}") }),
            );
        }
    };
    let cache = if file == hls::PLAYLIST {
        "no-cache"
    } else {
        "public, max-age=31536000, immutable"
    };
    (
        StatusCode::OK,
        [
            ("content-type", hls::content_type(&file)),
            ("cache-control", cache),
        ],
        Body::from(bytes),
    )
        .into_response()
}

/// DELETE /recordings/{filename} — remove this node's file, or the
/// named node's.
async fn delete_recording_handler(
    State(state): State<Arc<ConferenceState>>,
    headers: HeaderMap,
    Path(filename): Path<String>,
    Query(NodeParam { node }): Query<NodeParam>,
) -> (StatusCode, Json<serde_json::Value>) {
    match peer_for(&state, node.as_deref()) {
        PeerRoute::Local => {}
        PeerRoute::Unknown(node) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(
                    serde_json::json!({ "error": format!("Node {node} is not in cluster membership") }),
                ),
            );
        }
        PeerRoute::Peer(node) => {
            let Some(control) = &state.control else {
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(serde_json::json!({ "error": "This node is not clustered" })),
                );
            };
            if !forwardable_file_name(&filename) {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "error": "Invalid filename" })),
                );
            }
            let tenant = headers
                .get(PLATFORM_TENANT_HEADER)
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_string();
            return match control
                .send_to_node(
                    &node,
                    &tenant,
                    reqwest::Method::DELETE,
                    &format!("/recordings/{filename}"),
                    None,
                )
                .await
            {
                Some(outcome) => (
                    StatusCode::from_u16(outcome.status.unwrap_or(502))
                        .unwrap_or(StatusCode::BAD_GATEWAY),
                    Json(outcome.body),
                ),
                None => (
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(
                        serde_json::json!({ "error": format!("Node {node} is not in cluster membership") }),
                    ),
                ),
            };
        }
    }
    local_delete(&state, &filename).await
}

/// DELETE /internal/recordings/{filename} — what a peer asks this node for.
async fn internal_delete_recording_handler(
    State(state): State<Arc<ConferenceState>>,
    Path(filename): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    local_delete(&state, &filename).await
}

async fn local_delete(
    state: &ConferenceState,
    filename: &str,
) -> (StatusCode, Json<serde_json::Value>) {
    if let Some(dir) = stream_dir(state, filename) {
        if state.conference_handler.is_streaming(filename) {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({ "error": "The stream is still being written" })),
            );
        }
        return match tokio::fs::remove_dir_all(&dir).await {
            Ok(()) => {
                info!(sitting = %filename, "Stream deleted");
                (
                    StatusCode::OK,
                    Json(serde_json::json!({ "status": "deleted", "filename": filename })),
                )
            }
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Failed to delete the stream: {e}") })),
            ),
        };
    }
    let recordings_dir = state.conference_handler.recordings_dir();
    let Some(path) = safe_recording_path(&recordings_dir, filename) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Invalid filename" })),
        );
    };

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

/// Per-IP token-bucket rate limiter using DashMap.
///
/// Tracks request counts per source IP within a 1-second sliding window.
/// Evicts stale entries periodically to prevent unbounded memory growth.
struct PerIpRateLimiter {
    max_rps: u32,
    buckets: dashmap::DashMap<std::net::IpAddr, (u64, Instant)>,
}

impl PerIpRateLimiter {
    fn new(max_rps: u32) -> Self {
        Self {
            max_rps,
            buckets: dashmap::DashMap::new(),
        }
    }

    fn check(&self, ip: std::net::IpAddr) -> bool {
        let now = Instant::now();

        // Periodic eviction: if map > 1000 entries, prune stale ones
        if self.buckets.len() > 1000 {
            self.buckets.retain(|_, (_, window)| {
                now.duration_since(*window) < std::time::Duration::from_secs(60)
            });
        }

        let mut entry = self.buckets.entry(ip).or_insert((0, now));
        let (count, window_start) = entry.value_mut();
        if now.duration_since(*window_start) >= std::time::Duration::from_secs(1) {
            *count = 1;
            *window_start = now;
            true
        } else if *count < self.max_rps as u64 {
            *count += 1;
            true
        } else {
            false
        }
    }
}

async fn rate_limit_middleware(
    limiter: Arc<PerIpRateLimiter>,
    req: axum::http::Request<Body>,
    next: axum::middleware::Next,
) -> axum::response::Response {
    // Extract source IP from ConnectInfo or X-Forwarded-For
    let ip = req
        .extensions()
        .get::<axum::extract::ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0.ip())
        .or_else(|| {
            req.headers()
                .get("x-forwarded-for")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.split(',').next())
                .and_then(|s| s.trim().parse().ok())
        })
        .unwrap_or_else(|| {
            warn!("Rate limiter: unable to determine client IP, using 0.0.0.0");
            std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED)
        });

    if !limiter.check(ip) {
        return (StatusCode::TOO_MANY_REQUESTS, "Rate limit exceeded").into_response();
    }
    next.run(req).await
}

// =========================================================================
// Helpers
// =========================================================================

/// Validate a participant or room ID from URL path parameters.
/// Rejects empty, excessively long, or control-character-containing IDs.
fn is_valid_identifier(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id.bytes().all(|b| b > 0x1F && b != b'\x7F')
        && !id.contains('/')
        && !id.contains('\\')
}

/// Extract tenant ID from the authenticated control plane.
///
/// Conference HTTP APIs are scoped to the configured service tenant. The
/// control plane accepts the tenant header only when it matches that tenant.
async fn extract_tenant_id(
    headers: &HeaderMap,
    state: &ConferenceState,
) -> Result<TenantId, (StatusCode, Json<serde_json::Value>)> {
    let Some(raw_tenant_id) = headers
        .get(PLATFORM_TENANT_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(state.default_tenant_id);
    };

    let tenant_id = match raw_tenant_id.parse::<TenantId>() {
        Ok(tenant_id) => tenant_id,
        Err(e) => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": format!("Invalid {} header: {}", PLATFORM_TENANT_HEADER, e)
                })),
            ));
        }
    };

    if tenant_id == state.default_tenant_id {
        return Ok(tenant_id);
    }

    // Validate the requested tenant exists in the store.
    match state.storage.get_tenant(&tenant_id).await {
        Ok(Some(_)) => Ok(tenant_id),
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": format!("Tenant '{}' not found", tenant_id)
            })),
        )),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("Failed to resolve tenant: {}", e)
            })),
        )),
    }
}

async fn room_number_exists_in_tenant(
    state: &ConferenceState,
    tenant_id: &TenantId,
    room_number: &str,
) -> anyhow::Result<bool> {
    state
        .storage
        .get_meeting_by_room_number(tenant_id, room_number)
        .await
        .map(|meeting| meeting.is_some())
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
        "owner_id": meeting.owner_id,
        "pin_prompt_sound": meeting.pin_prompt_sound,
        "invalid_pin_sound": meeting.invalid_pin_sound,
        "moh_sound": meeting.moh_sound,
        "pin_prompt_tts": meeting.pin_prompt_tts,
        "invalid_pin_tts": meeting.invalid_pin_tts,
        "welcome_tts": meeting.welcome_tts,
        "play_entry_tone": meeting.play_entry_tone,
        "play_exit_tone": meeting.play_exit_tone,
        "join_sound": meeting.join_sound,
        "exit_sound": meeting.exit_sound,
        "video_enabled": meeting.video_enabled,
        "video_layout": meeting.video_layout,
        "video_max_participants": meeting.video_max_participants,
        "video_max_resolution": meeting.video_max_resolution,
        "video_fps": meeting.video_fps,
        "video_exclude_self": meeting.video_exclude_self,
        "video_codecs": meeting.video_codecs,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::video_bridge::VideoControlError;

    /// A name that goes into a peer's URL may be a file name and nothing
    /// else, so nothing a caller sends can change the request the peer
    /// sees.
    #[test]
    fn only_a_bare_file_name_is_forwarded_to_a_peer() {
        assert!(forwardable_file_name("8001-1788800000-abc123.webm"));
        assert!(forwardable_file_name("8001-1788800000-abc123.mp4"));
        assert!(forwardable_file_name("room.wav"));
        for bad in [
            "",
            "../../etc/passwd",
            "a/b.wav",
            "a\\b.wav",
            "x.wav?node=other",
            "x.wav#frag",
            "x.wav&a=b",
            "space name.wav",
            "%2e%2e.wav",
        ] {
            assert!(!forwardable_file_name(bad), "{bad}");
        }
        assert!(!forwardable_file_name(&"a".repeat(256)));
    }

    /// "Not here" is the only answer that sends the request on to a peer;
    /// a room without video or a bad layout is decided on this node.
    #[test]
    fn video_control_errors_map_to_forwardable_and_final_statuses() {
        assert_eq!(
            video_control_status(&VideoControlError::NoSuchParticipant("p".into())),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            video_control_status(&VideoControlError::NoVideo),
            StatusCode::CONFLICT
        );
        assert_eq!(
            video_control_status(&VideoControlError::UnknownLayout("x".into())),
            StatusCode::BAD_REQUEST
        );
    }

    #[test]
    fn participant_video_state_is_on_or_off() {
        assert_eq!(parse_video_state("on"), Some(true));
        assert_eq!(parse_video_state("off"), Some(false));
        assert_eq!(parse_video_state("On"), None);
        assert_eq!(parse_video_state("mute"), None);
    }
}
