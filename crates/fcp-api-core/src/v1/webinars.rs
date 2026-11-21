//! A webinar's registrations and attendance (`docs/WEBINARS.md` §2.6,
//! W4.2), under `/conferences/meetings/:id`.
//!
//! - **Registrants.** An administrator registers a person for a webinar
//!   and is handed their attendee token (and join link), which names the
//!   registration. The room admits the token while the registration is not
//!   revoked; a webinar whose `registration` is set admits no attendee any
//!   other way. A token can be minted again, for the registrant who lost
//!   theirs, and a registration revoked.
//! - **Sittings.** Each run of the meeting's room, newest first.
//! - **Attendance.** One sitting read back: who registered and who came,
//!   each leg from its CDR (role, registration, when, how long, whether
//!   they were only ever in the lobby), with the sitting's questions and
//!   chat.

use crate::auth::TenantContext;
use crate::error::ApiError;
use crate::state::ApiState;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use fcp_core::join_token::MAX_REGISTRANT_TOKEN_TTL;
use fcp_core::storage::{CdrKind, CdrQuery, CdrStore, ConferenceStore};
use fcp_core::webinar::{Registrant, Sitting, MAX_REGISTRANTS};
use fcp_core::{JoinRole, JoinToken, Meeting, MeetingId};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tracing::info;

macro_rules! require_storage {
    ($state:expr) => {
        $state.storage.as_ref().ok_or_else(|| {
            ApiError::NotFound("Storage is required but not configured".to_string())
        })?
    };
}

/// The default lifetime of a registrant's token: thirty days.
pub const DEFAULT_REGISTRANT_TTL_SECS: u64 = 30 * 24 * 60 * 60;

/// The most legs, questions and messages one attendance report reads.
const MAX_REPORT_ROWS: usize = 20_000;

/// The meeting `id` in the tenant.
pub(crate) async fn meeting_of(
    state: &ApiState,
    tenant: &TenantContext,
    id: &str,
) -> Result<Meeting, ApiError> {
    let storage = require_storage!(state);
    let meeting_id = MeetingId::from_str(id)
        .map_err(|e| ApiError::BadRequest(format!("Invalid meeting ID: {}", e)))?;
    let meeting = storage
        .get_meeting(&meeting_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .filter(|m| m.tenant_id == tenant.0)
        .ok_or_else(|| ApiError::NotFound(format!("Meeting '{}' not found", id)))?;
    Ok(meeting)
}

/// The meeting `id` in the tenant, when it is a webinar.
pub(crate) async fn webinar_of(
    state: &ApiState,
    tenant: &TenantContext,
    id: &str,
) -> Result<Meeting, ApiError> {
    let meeting = meeting_of(state, tenant, id).await?;
    if !meeting.is_webinar() {
        return Err(ApiError::Conflict(format!(
            "meeting '{id}' is a meeting, not a webinar"
        )));
    }
    Ok(meeting)
}

fn registrant_store(
    state: &ApiState,
) -> Result<Arc<dyn fcp_core::webinar::RegistrantStore>, ApiError> {
    state
        .storage
        .as_ref()
        .and_then(|s| s.registrant_store().cloned())
        .ok_or_else(|| ApiError::ServiceUnavailable("registrations are not kept here".into()))
}

/// The registration `rid` of `meeting`.
async fn registrant_of(
    state: &ApiState,
    meeting: &Meeting,
    rid: &str,
) -> Result<Registrant, ApiError> {
    registrant_store(state)?
        .get_registrant(&meeting.tenant_id, rid)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .filter(|r| r.meeting_id == meeting.id.to_string())
        .ok_or_else(|| ApiError::NotFound(format!("Registration '{rid}' not found")))
}

/// A registration as the API shows it.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct RegistrantResponse {
    pub id: String,
    pub meeting_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(with = "crate::time::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub created_at: SystemTime,
    /// When it was revoked: its token is refused.
    #[serde(with = "crate::time::rfc3339_opt")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub revoked_at: Option<SystemTime>,
}

impl From<Registrant> for RegistrantResponse {
    fn from(r: Registrant) -> Self {
        Self {
            id: r.id,
            meeting_id: r.meeting_id,
            name: r.name,
            email: r.email,
            created_at: r.created_at,
            revoked_at: r.revoked_at,
        }
    }
}

/// A registration and the token minted for it.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct RegisteredResponse {
    pub registrant: RegistrantResponse,
    /// The attendee token naming the registration.
    pub token: super::join_tokens::JoinTokenResponse,
}

/// `POST …/registrants` body.
#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RegisterRequest {
    /// The name the room shows them by (at most 64 characters).
    pub name: String,
    #[serde(default)]
    pub email: Option<String>,
    /// The token's lifetime in seconds: thirty days by default, ninety at
    /// most.
    #[serde(default)]
    pub ttl_secs: Option<u64>,
}

/// `POST …/registrants/:rid/token` body.
#[derive(Debug, Clone, Default, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RegistrantTokenRequest {
    #[serde(default)]
    pub ttl_secs: Option<u64>,
}

/// Mint `registrant`'s attendee token.
fn mint_for_registrant(
    state: &ApiState,
    meeting: &Meeting,
    registrant: &Registrant,
    ttl_secs: Option<u64>,
) -> Result<super::join_tokens::JoinTokenResponse, ApiError> {
    let secret = state
        .conference_join_secret
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            ApiError::ServiceUnavailable(
                "join tokens are not configured: set [conference] join_secret".to_string(),
            )
        })?;
    let ttl_secs = ttl_secs.unwrap_or(DEFAULT_REGISTRANT_TTL_SECS);
    if ttl_secs == 0 || ttl_secs > MAX_REGISTRANT_TOKEN_TTL.as_secs() {
        return Err(ApiError::BadRequest(format!(
            "ttl_secs must be between 1 and {}",
            MAX_REGISTRANT_TOKEN_TTL.as_secs()
        )));
    }
    let token = JoinToken::new(
        meeting.tenant_id,
        meeting.room_number.clone(),
        registrant.name.clone(),
        JoinRole::Attendee,
        Duration::from_secs(ttl_secs),
    )
    .for_registrant(registrant.id.clone());
    let wire = token.mint(secret.as_bytes());
    let join_url = state
        .conference_gateway_url
        .as_deref()
        .filter(|u| !u.is_empty())
        .map(|u| super::join_tokens::join_url(u, &wire, &meeting.room_number, &registrant.name));
    Ok(super::join_tokens::JoinTokenResponse {
        token: wire,
        room_number: meeting.room_number.clone(),
        display_name: registrant.name.clone(),
        role: JoinRole::Attendee,
        expires_at: token.expires_at,
        join_url,
    })
}

/// POST /conferences/meetings/:id/registrants — register a person for a
/// webinar: their registration and attendee token.
#[utoipa::path(
    post,
    path = "/api/v1/conferences/meetings/{id}/registrants",
    tag = "conferences",
    params(("id" = String, Path)),
    request_body = RegisterRequest,
    responses(
        (status = 201, description = "Registered", body = RegisteredResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn register(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(id): Path<String>,
    Json(req): Json<RegisterRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let meeting = webinar_of(&state, &tenant, &id).await?;
    let name = req.name.trim().to_string();
    if name.is_empty() || name.chars().count() > 64 {
        return Err(ApiError::BadRequest(
            "name must be 1 to 64 characters".to_string(),
        ));
    }
    let email = req
        .email
        .map(|e| e.trim().to_string())
        .filter(|e| !e.is_empty());
    if let Some(e) = &email {
        if e.len() > 254 || !e.contains('@') {
            return Err(ApiError::BadRequest("email is not an address".to_string()));
        }
    }
    let store = registrant_store(&state)?;
    let (_, total) = store
        .registrants_of_meeting(&meeting.tenant_id, &meeting.id.to_string(), None, 0)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if total as usize >= MAX_REGISTRANTS {
        return Err(ApiError::Conflict(format!(
            "a webinar has at most {MAX_REGISTRANTS} registrants"
        )));
    }
    let registrant = Registrant::new(meeting.tenant_id, meeting.id.to_string(), name, email);
    // The token first: a webinar the gateway cannot mint for has nobody
    // registered for nothing.
    let token = mint_for_registrant(&state, &meeting, &registrant, req.ttl_secs)?;
    store
        .put_registrant(&registrant)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    info!(meeting = %meeting.id, registrant = %registrant.id, "A webinar registration");
    Ok((
        StatusCode::CREATED,
        Json(RegisteredResponse {
            registrant: registrant.into(),
            token,
        }),
    ))
}

/// GET /conferences/meetings/:id/registrants — a webinar's registrations,
/// oldest first.
#[utoipa::path(
    get,
    path = "/api/v1/conferences/meetings/{id}/registrants",
    tag = "conferences",
    params(("id" = String, Path), crate::page::PageQuery),
    responses(
        (status = 200, description = "OK", body = crate::page::Page<RegistrantResponse>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn registrants(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(id): Path<String>,
    Query(page): Query<crate::page::PageQuery>,
) -> Result<Json<crate::page::Page<RegistrantResponse>>, ApiError> {
    let meeting = webinar_of(&state, &tenant, &id).await?;
    let limit = page.limit()?;
    let after = page.after()?;
    let (list, total) = registrant_store(&state)?
        .registrants_of_meeting(
            &meeting.tenant_id,
            &meeting.id.to_string(),
            after.as_deref(),
            limit + 1,
        )
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let more = list.len() > limit;
    let data: Vec<RegistrantResponse> = list.into_iter().take(limit).map(Into::into).collect();
    let next = more.then(|| data.last().map(|r| r.id.clone())).flatten();
    Ok(Json(crate::page::Page::new(data, next, Some(total))))
}

/// GET /conferences/meetings/:id/registrants/:rid — one registration.
#[utoipa::path(
    get,
    path = "/api/v1/conferences/meetings/{id}/registrants/{rid}",
    tag = "conferences",
    params(("id" = String, Path), ("rid" = String, Path)),
    responses(
        (status = 200, description = "OK", body = RegistrantResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn registrant(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path((id, rid)): Path<(String, String)>,
) -> Result<Json<RegistrantResponse>, ApiError> {
    let meeting = webinar_of(&state, &tenant, &id).await?;
    Ok(Json(registrant_of(&state, &meeting, &rid).await?.into()))
}

/// POST /conferences/meetings/:id/registrants/:rid/token — mint a
/// registrant's attendee token again (the old one keeps working until it
/// expires or the registration is revoked).
#[utoipa::path(
    post,
    path = "/api/v1/conferences/meetings/{id}/registrants/{rid}/token",
    tag = "conferences",
    params(("id" = String, Path), ("rid" = String, Path)),
    request_body = RegistrantTokenRequest,
    responses(
        (status = 201, description = "Minted", body = super::join_tokens::JoinTokenResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn registrant_token(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path((id, rid)): Path<(String, String)>,
    body: Option<Json<RegistrantTokenRequest>>,
) -> Result<impl IntoResponse, ApiError> {
    let meeting = webinar_of(&state, &tenant, &id).await?;
    let registrant = registrant_of(&state, &meeting, &rid).await?;
    if !registrant.stands() {
        return Err(ApiError::Conflict(format!(
            "registration '{rid}' is revoked"
        )));
    }
    let req = body.map(|Json(b)| b).unwrap_or_default();
    let token = mint_for_registrant(&state, &meeting, &registrant, req.ttl_secs)?;
    Ok((StatusCode::CREATED, Json(token)))
}

/// DELETE /conferences/meetings/:id/registrants/:rid — revoke a
/// registration: its token is refused from now on. Someone already in
/// the room stays until a host removes them.
#[utoipa::path(
    delete,
    path = "/api/v1/conferences/meetings/{id}/registrants/{rid}",
    tag = "conferences",
    params(("id" = String, Path), ("rid" = String, Path)),
    responses(
        (status = 200, description = "Revoked", body = RegistrantResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn revoke_registrant(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path((id, rid)): Path<(String, String)>,
) -> Result<Json<RegistrantResponse>, ApiError> {
    let meeting = webinar_of(&state, &tenant, &id).await?;
    let mut registrant = registrant_of(&state, &meeting, &rid).await?;
    if registrant.revoked_at.is_none() {
        registrant.revoked_at = Some(SystemTime::now());
        registrant_store(&state)?
            .put_registrant(&registrant)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        info!(meeting = %meeting.id, registrant = %rid, "A webinar registration revoked");
    }
    Ok(Json(registrant.into()))
}

/// One sitting of a meeting's room as the API shows it.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct SittingResponse {
    pub id: String,
    pub room_number: String,
    /// The meeting's kind when it began.
    pub kind: String,
    #[serde(with = "crate::time::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub started_at: SystemTime,
    /// When it went live; a meeting's is its start, a webinar that never
    /// went live has none.
    #[serde(with = "crate::time::rfc3339_opt")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub live_at: Option<SystemTime>,
    /// When it ended; none while it runs (or when its node stopped
    /// before it could say).
    #[serde(with = "crate::time::rfc3339_opt")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub ended_at: Option<SystemTime>,
    /// Who ended a webinar: a host's participant id, `host`, `api` or
    /// `stage_left`; none when its last caller left.
    pub ended_by: Option<String>,
}

impl From<Sitting> for SittingResponse {
    fn from(s: Sitting) -> Self {
        Self {
            id: s.id,
            room_number: s.room_number,
            kind: s.kind,
            started_at: s.started_at,
            live_at: s.live_at,
            ended_at: s.ended_at,
            ended_by: s.ended_by,
        }
    }
}

fn sitting_store(state: &ApiState) -> Result<Arc<dyn fcp_core::webinar::SittingStore>, ApiError> {
    state
        .storage
        .as_ref()
        .and_then(|s| s.sitting_store().cloned())
        .ok_or_else(|| ApiError::ServiceUnavailable("sittings are not kept here".into()))
}

/// GET /conferences/meetings/:id/sittings — each run of the meeting's
/// room, newest first.
#[utoipa::path(
    get,
    path = "/api/v1/conferences/meetings/{id}/sittings",
    tag = "conferences",
    params(("id" = String, Path), crate::page::PageQuery),
    responses(
        (status = 200, description = "OK", body = crate::page::Page<SittingResponse>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn sittings(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(id): Path<String>,
    Query(page): Query<crate::page::PageQuery>,
) -> Result<Json<crate::page::Page<SittingResponse>>, ApiError> {
    let meeting = meeting_of(&state, &tenant, &id).await?;
    let list = sitting_store(&state)?
        .sittings_of_meeting(&meeting.tenant_id, &meeting.id.to_string(), MAX_REPORT_ROWS)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let items: Vec<SittingResponse> = list.into_iter().map(Into::into).collect();
    Ok(Json(crate::page::paginate(
        items,
        &page,
        crate::page::Order::Descending,
        |s| crate::page::system_time_key(s.started_at, &s.id),
    )?))
}

/// `?sitting=` of the attendance report.
#[derive(Debug, Clone, Default, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct AttendanceQuery {
    /// The sitting; the newest when absent.
    pub sitting: Option<String>,
}

/// One leg of a sitting, from its record.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct AttendanceLeg {
    pub participant_id: Option<String>,
    pub name: Option<String>,
    /// Where they called from.
    pub from: String,
    /// `host`, `panelist`, `attendee` or `guest`.
    pub role: Option<String>,
    /// The registration they came by.
    pub registrant: Option<String>,
    #[serde(with = "crate::time::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub joined_at: SystemTime,
    #[serde(with = "crate::time::rfc3339_opt")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub left_at: Option<SystemTime>,
    pub duration_secs: u64,
    /// They left from the lobby, never let in.
    pub lobby: bool,
    /// Questions they asked, and chat messages they posted, in the sitting.
    pub questions: u32,
    pub messages: u32,
}

/// A registration and whether they came.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct AttendanceRegistrant {
    #[serde(flatten)]
    pub registrant: RegistrantResponse,
    pub attended: bool,
    /// Their time in the room across legs (the lobby aside).
    pub attended_secs: u64,
}

/// A question of the sitting, as kept.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct AttendanceQuestion {
    pub id: String,
    pub participant_id: String,
    pub asker: Option<String>,
    pub text: String,
    pub status: fcp_core::qa::QuestionStatus,
    pub published: bool,
    pub upvotes: u32,
    pub answer: Option<String>,
    pub answered_live: bool,
    #[serde(with = "crate::time::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub asked_at: SystemTime,
}

/// A chat message of the sitting, as kept.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct AttendanceMessage {
    pub id: String,
    pub participant_id: String,
    pub sender: Option<String>,
    /// `room` or `hosts`.
    pub audience: fcp_core::chat::ChatAudience,
    pub text: String,
    #[serde(with = "crate::time::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub sent_at: SystemTime,
    #[serde(with = "crate::time::rfc3339_opt")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub deleted_at: Option<SystemTime>,
}

/// Who came to one sitting.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct AttendanceResponse {
    pub sitting: SittingResponse,
    /// Distinct people in the room (by registration, else by where they
    /// called from), the lobby aside.
    pub attendees: u64,
    /// The most legs in the room at once.
    pub peak: u64,
    pub legs: Vec<AttendanceLeg>,
    /// A webinar's registrations, each with whether they came; empty for a
    /// meeting.
    pub registrants: Vec<AttendanceRegistrant>,
    pub questions: Vec<AttendanceQuestion>,
    pub messages: Vec<AttendanceMessage>,
}

/// The most legs in the room at once.
fn peak_of(legs: &[AttendanceLeg]) -> u64 {
    let mut edges: Vec<(SystemTime, i64)> = Vec::new();
    for l in legs.iter().filter(|l| !l.lobby) {
        edges.push((l.joined_at, 1));
        edges.push((l.left_at.unwrap_or_else(SystemTime::now), -1));
    }
    // Leaving before joining at the same instant.
    edges.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    let (mut now, mut peak) = (0i64, 0i64);
    for (_, d) in edges {
        now += d;
        peak = peak.max(now);
    }
    peak.max(0) as u64
}

/// GET /conferences/meetings/:id/attendance — who registered for and came
/// to one sitting of the meeting's room, with its questions and chat.
#[utoipa::path(
    get,
    path = "/api/v1/conferences/meetings/{id}/attendance",
    tag = "conferences",
    params(("id" = String, Path), AttendanceQuery),
    responses(
        (status = 200, description = "OK", body = AttendanceResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn attendance(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(id): Path<String>,
    Query(q): Query<AttendanceQuery>,
) -> Result<Json<AttendanceResponse>, ApiError> {
    let meeting = meeting_of(&state, &tenant, &id).await?;
    let storage = require_storage!(state);
    let sittings = sitting_store(&state)?;
    let sitting = match q.sitting.as_deref().filter(|s| !s.is_empty()) {
        Some(sid) => sittings
            .get_sitting(&meeting.tenant_id, sid)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
            .filter(|s| s.meeting_id.as_deref() == Some(meeting.id.to_string().as_str())),
        None => sittings
            .sittings_of_meeting(&meeting.tenant_id, &meeting.id.to_string(), 1)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
            .into_iter()
            .next(),
    }
    .ok_or_else(|| ApiError::NotFound("no such sitting of this meeting".to_string()))?;

    // The sitting's legs: conference records naming it, from its start.
    let records = storage
        .query_cdrs(CdrQuery {
            tenant_id: Some(meeting.tenant_id),
            kind: Some(CdrKind::Conference),
            start_time_from: Some(sitting.started_at - Duration::from_secs(60)),
            custom_data: Some(serde_json::json!({ "sitting_id": sitting.id })),
            limit: Some(MAX_REPORT_ROWS),
            ..Default::default()
        })
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let questions = match storage.question_store() {
        Some(s) => s
            .questions_of_session(&meeting.tenant_id, &sitting.id, MAX_REPORT_ROWS)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?,
        None => Vec::new(),
    };
    let messages = match storage.chat_store() {
        Some(s) => s
            .chat_of_session(&meeting.tenant_id, &sitting.id, MAX_REPORT_ROWS)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?,
        None => Vec::new(),
    };
    let mut asked: HashMap<&str, u32> = HashMap::new();
    for q in &questions {
        *asked.entry(q.participant_id.as_str()).or_default() += 1;
    }
    let mut posted: HashMap<&str, u32> = HashMap::new();
    for m in &messages {
        *posted.entry(m.participant_id.as_str()).or_default() += 1;
    }

    let mut legs: Vec<AttendanceLeg> = records
        .iter()
        .filter(|r| {
            // Trunks between nodes are not people.
            r.custom_data
                .as_ref()
                .and_then(|c| c.get("trunk_peer"))
                .is_none_or(|v| v.is_null())
        })
        .map(|r| {
            let c = r.custom_data.clone().unwrap_or_default();
            let text = |k: &str| c.get(k).and_then(|v| v.as_str()).map(str::to_string);
            let pid = text("participant_id");
            let joined_at = r.answer_time.unwrap_or(r.start_time);
            let left_at = r.end_time;
            let duration_secs = left_at
                .and_then(|l| l.duration_since(joined_at).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            AttendanceLeg {
                questions: pid
                    .as_deref()
                    .and_then(|p| asked.get(p))
                    .copied()
                    .unwrap_or(0),
                messages: pid
                    .as_deref()
                    .and_then(|p| posted.get(p))
                    .copied()
                    .unwrap_or(0),
                participant_id: pid,
                name: r.from_display_name.clone(),
                from: r.from_uri.clone(),
                role: text("role"),
                registrant: text("registrant"),
                joined_at,
                left_at,
                duration_secs,
                lobby: c.get("lobby").and_then(|v| v.as_bool()).unwrap_or(false),
            }
        })
        .collect();
    legs.sort_by_key(|l| l.joined_at);

    let in_room: Vec<&AttendanceLeg> = legs.iter().filter(|l| !l.lobby).collect();
    let attendees = in_room
        .iter()
        .map(|l| l.registrant.clone().unwrap_or_else(|| l.from.clone()))
        .collect::<HashSet<_>>()
        .len() as u64;
    let mut time_of: BTreeMap<&str, u64> = BTreeMap::new();
    for l in &in_room {
        if let Some(r) = l.registrant.as_deref() {
            *time_of.entry(r).or_default() += l.duration_secs;
        }
    }

    let registrants = if meeting.is_webinar() {
        let (list, _) = registrant_store(&state)?
            .registrants_of_meeting(
                &meeting.tenant_id,
                &meeting.id.to_string(),
                None,
                MAX_REGISTRANTS,
            )
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        list.into_iter()
            .map(|r| {
                let secs = time_of.get(r.id.as_str()).copied();
                AttendanceRegistrant {
                    attended: secs.is_some(),
                    attended_secs: secs.unwrap_or(0),
                    registrant: r.into(),
                }
            })
            .collect()
    } else {
        Vec::new()
    };

    Ok(Json(AttendanceResponse {
        peak: peak_of(&legs),
        attendees,
        sitting: sitting.into(),
        registrants,
        questions: questions
            .into_iter()
            .map(|q| AttendanceQuestion {
                id: q.id,
                participant_id: q.participant_id,
                asker: q.asker_name,
                text: q.text,
                status: q.status,
                published: q.published,
                upvotes: q.upvotes,
                answer: q.answer,
                answered_live: q.answered_live,
                asked_at: q.asked_at,
            })
            .collect(),
        messages: messages
            .into_iter()
            .map(|m| AttendanceMessage {
                id: m.id,
                participant_id: m.participant_id,
                sender: m.sender_name,
                audience: m.audience,
                text: m.text,
                sent_at: m.sent_at,
                deleted_at: m.deleted_at,
            })
            .collect(),
        legs,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leg(join: u64, leave: Option<u64>, lobby: bool) -> AttendanceLeg {
        let at = |s: u64| SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000 + s);
        AttendanceLeg {
            participant_id: None,
            name: None,
            from: String::new(),
            role: None,
            registrant: None,
            joined_at: at(join),
            left_at: leave.map(at),
            duration_secs: 0,
            lobby,
            questions: 0,
            messages: 0,
        }
    }

    #[test]
    fn the_peak_counts_legs_in_the_room_at_once_and_not_the_lobby() {
        let legs = [
            leg(0, Some(100), false),
            leg(10, Some(50), false),
            leg(50, Some(60), false),
            leg(20, Some(30), true),
        ];
        assert_eq!(peak_of(&legs), 2);
        assert_eq!(peak_of(&[]), 0);
    }
}
