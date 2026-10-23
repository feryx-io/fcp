//! Live call control on the node (`docs/DEVELOPER_SURFACE.md` phase 8):
//! `/calls/:id/{hold,resume,transfer,redirect,play,say,hangup}`, which the
//! admin API forwards to the node holding the call; and the calls the node
//! places for others: a phone's resync, a test push, an emergency
//! notification call. Behind the node's API
//! token, like the supervision routes, and scoped to the tenant the caller
//! names (the node's default tenant otherwise).

use crate::health::{scoped_call, AppState};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use fcp_core::TenantId;
use fcp_sip::call_control::{parse_party, CallControlError};
use serde::Deserialize;
use std::sync::Arc;

/// The routes, for the protected router.
pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/calls/:id/hold", post(hold))
        .route("/calls/:id/resume", post(resume))
        .route("/calls/:id/transfer", post(transfer))
        .route("/calls/:id/redirect", post(redirect))
        .route("/calls/:id/play", post(play))
        .route("/calls/:id/say", post(say))
        .route("/calls/:id/hangup", post(hangup))
        .route("/devices/:id/resync", post(resync_device))
        .route("/devices/:id/push-test", post(push_test_device))
        .route("/push", post(relayed_push))
        .route("/emergency/notify-call", post(notify_call))
}

/// What a notification call is asked to do (`fcp_notify::call`).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NotifyCallBody {
    /// One of the tenant's users or extensions, or a SIP URI.
    to: String,
    /// What is spoken once it is answered.
    text: String,
    #[serde(default)]
    tenant_id: Option<TenantId>,
    /// How long to ring, in seconds (5 to 120; 30 when unsaid).
    #[serde(default)]
    ring_timeout_secs: Option<u64>,
    /// How often the notice is spoken (1 to 5; twice when unsaid).
    #[serde(default)]
    repeat: Option<u32>,
    /// Caller ID, the emergency call it is about, and whether that was a
    /// test: for the log.
    #[serde(default)]
    from: Option<String>,
    #[serde(default)]
    call_id: Option<String>,
    #[serde(default)]
    test: bool,
    /// The contact may listen in to the emergency call (`call_id`) once
    /// told: they are asked, and joined to it if they choose to.
    #[serde(default)]
    listen: bool,
}

/// POST /emergency/notify-call — ring a contact and speak an emergency
/// notice to them (`docs/E911_BLOCK_2.md` §8.1): the events server's call
/// channel, which has no SIP stack. What became of the call comes back;
/// only `answered` told anybody anything.
async fn notify_call(
    State(state): State<Arc<AppState>>,
    Json(body): Json<NotifyCallBody>,
) -> Response {
    let (to, text) = (body.to.trim(), body.text.trim());
    if to.is_empty() || text.is_empty() {
        return error(CallControlError::bad_request(
            "a notification call needs `to` and `text`",
        ));
    }
    let Some(handler) = state.sip_handler.clone() else {
        return error(CallControlError::new(
            503,
            "notification calls are not available on this node",
        ));
    };
    let tenant = body.tenant_id.unwrap_or(state.default_tenant_id);
    let ring = std::time::Duration::from_secs(body.ring_timeout_secs.unwrap_or(30).clamp(5, 120));
    let repeat = body.repeat.unwrap_or(2).clamp(1, 5);
    let listen_to = body
        .call_id
        .as_deref()
        .filter(|_| body.listen)
        .and_then(|id| id.trim_start_matches("call-").parse::<uuid::Uuid>().ok())
        .map(fcp_b2bua_core::CallId::from_uuid);
    match handler
        .announce_call(&tenant, to, text, ring, repeat, listen_to)
        .await
    {
        Ok(outcome) => {
            tracing::info!(
                tenant_id = %tenant, to, from = ?body.from, call_id = ?body.call_id,
                test = body.test, listen = body.listen, outcome = %outcome,
                "Emergency notification call"
            );
            (
                StatusCode::OK,
                Json(serde_json::json!({ "outcome": outcome.to_string(), "to": to })),
            )
                .into_response()
        }
        Err(e) => error(CallControlError::new(
            503,
            format!("the call could not be placed: {e}"),
        )),
    }
}

/// POST /push — a site's push, sent with this node's keys (`docs/PUSH.md`
/// §2.4). The admin's link listener forwards it here, having held the
/// message's tenant to the site's. 503 from a node without keys, which the
/// admin passes over.
async fn relayed_push(
    State(state): State<Arc<AppState>>,
    Json(push): Json<fcp_core::push::RelayedPush>,
) -> Response {
    let outcome = match &state.sip_handler {
        Some(handler) => handler.send_relayed_push(&push).await,
        None => None,
    };
    match outcome {
        Some(Ok(outcome)) => (StatusCode::OK, Json(outcome.answer())).into_response(),
        Some(Err(e)) => error(CallControlError::new(400, e)),
        None => error(CallControlError::new(
            503,
            "push is not configured on this node",
        )),
    }
}

/// Who a test push is for.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PushTestBody {
    #[serde(default)]
    tenant_id: Option<TenantId>,
}

/// POST /devices/:id/push-test — a `test` push to each of the device's
/// bindings that can be woken (`docs/PUSH.md` §2.3): what each did.
async fn push_test_device(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    body: Option<Json<PushTestBody>>,
) -> Response {
    use fcp_core::storage::DeviceStore;
    let tenant = body
        .and_then(|Json(b)| b.tenant_id)
        .unwrap_or(state.default_tenant_id);
    let not_found = || {
        error(CallControlError::new(
            404,
            format!("Device not found: {id}"),
        ))
    };
    let Ok(device_id) = id.parse::<fcp_core::DeviceId>() else {
        return not_found();
    };
    let device = match state.storage.get_device(&device_id).await {
        Ok(Some(d)) if d.tenant_id == tenant => d,
        Ok(_) => return not_found(),
        Err(e) => return error(CallControlError::new(500, e.to_string())),
    };
    let results = match &state.sip_handler {
        Some(handler) => handler.push_test_device(&device).await,
        None => None,
    };
    match results {
        Some(results) => (
            StatusCode::OK,
            Json(serde_json::json!({"device_id": id, "results": results})),
        )
            .into_response(),
        None => error(CallControlError::new(
            503,
            "push is not configured on this node",
        )),
    }
}

/// Who a resync is for.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResyncBody {
    #[serde(default)]
    tenant_id: Option<TenantId>,
    /// Also send to contacts with no flow; the admin asks one node to.
    /// Unset: yes, as a node asked alone must.
    #[serde(default)]
    flowless: Option<bool>,
}

/// POST /devices/:id/resync — tell the device's phone to fetch its
/// configuration again (`docs/PROVISIONING.md` §8): its vendor's NOTIFY to
/// each of its registered contacts. `sent` is how many.
async fn resync_device(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    body: Option<Json<ResyncBody>>,
) -> Response {
    use fcp_core::storage::DeviceStore;
    let body = body.map(|Json(b)| b).unwrap_or_default();
    let tenant = body.tenant_id.unwrap_or(state.default_tenant_id);
    let not_found = || {
        error(CallControlError::new(
            404,
            format!("Device not found: {id}"),
        ))
    };
    let Ok(device_id) = id.parse::<fcp_core::DeviceId>() else {
        return not_found();
    };
    let device = match state.storage.get_device(&device_id).await {
        Ok(Some(d)) if d.tenant_id == tenant => d,
        Ok(_) => return not_found(),
        Err(e) => return error(CallControlError::new(500, e.to_string())),
    };
    let Some(handler) = state.sip_handler.clone() else {
        return error(CallControlError::new(
            503,
            "resync is not available on this node",
        ));
    };
    match handler
        .resync_device(&device, body.flowless.unwrap_or(true))
        .await
    {
        Some(sent) => (
            StatusCode::OK,
            Json(serde_json::json!({"device_id": id, "sent": sent})),
        )
            .into_response(),
        None => error(CallControlError::new(
            503,
            "resync is not available on this node",
        )),
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Body {
    /// The call's tenant, when the caller scopes it (the admin API does).
    #[serde(default)]
    tenant_id: Option<TenantId>,
    /// Who is held, transferred or redirected: `caller` (the default) or
    /// `callee`.
    #[serde(default)]
    party: Option<String>,
    /// Transfer: who to connect the party to.
    #[serde(default)]
    target: Option<String>,
    /// Redirect: the flow the party is moved into.
    #[serde(default)]
    flow_id: Option<String>,
    /// Play: the tenant's prompt.
    #[serde(default)]
    prompt_id: Option<String>,
    /// Say: the text.
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    voice: Option<String>,
    /// Play and say: who hears it — `both` (the default), `caller` or
    /// `callee`.
    #[serde(default)]
    to: Option<String>,
}

fn error(e: CallControlError) -> Response {
    let status = StatusCode::from_u16(e.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (status, Json(serde_json::json!({ "error": e.message }))).into_response()
}

/// A handler's outcome as the node answers it.
fn answer(outcome: Result<(StatusCode, serde_json::Value), CallControlError>) -> Response {
    match outcome {
        Ok((status, body)) => (status, Json(body)).into_response(),
        Err(e) => error(e),
    }
}

type Outcome = Result<(StatusCode, serde_json::Value), CallControlError>;

/// The call, the handler that controls it, and the body's tenant checked.
fn call(
    state: &AppState,
    id: &str,
    body: &Body,
) -> Result<
    (
        fcp_b2bua_core::CallId,
        Arc<fcp_sip::handler::SipMessageHandler>,
    ),
    CallControlError,
> {
    let handler = state
        .sip_handler
        .clone()
        .ok_or_else(|| CallControlError::new(503, "call control is not available on this node"))?;
    let call_id =
        scoped_call(state, id, body.tenant_id).ok_or_else(|| CallControlError::not_found(id))?;
    Ok((call_id, handler))
}

fn party(body: &Body) -> Result<fcp_b2bua_core::LegSide, CallControlError> {
    parse_party(body.party.as_deref().unwrap_or("caller"))
}

fn audience(body: &Body) -> Result<fcp_media::AudioTarget, CallControlError> {
    match body.to.as_deref().map(str::to_ascii_lowercase).as_deref() {
        None | Some("both") => Ok(fcp_media::AudioTarget::Both),
        Some(name) => Ok(match parse_party(name)? {
            fcp_b2bua_core::LegSide::A => fcp_media::AudioTarget::ParticipantA,
            fcp_b2bua_core::LegSide::B => fcp_media::AudioTarget::ParticipantB,
        }),
    }
}

async fn hold(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    body: Option<Json<Body>>,
) -> Response {
    let body = body.map(|Json(b)| b).unwrap_or_default();
    answer((|| {
        let (call_id, handler) = call(&state, &id, &body)?;
        let party = party(&body)?;
        handler.hold_call(&call_id, party)?;
        Ok((
            StatusCode::OK,
            serde_json::json!({"call_id": id, "status": "held", "party": party_name(party)}),
        ))
    })())
}

async fn resume(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    body: Option<Json<Body>>,
) -> Response {
    let body = body.map(|Json(b)| b).unwrap_or_default();
    answer((|| {
        let (call_id, handler) = call(&state, &id, &body)?;
        handler.resume_call(&call_id)?;
        Ok((
            StatusCode::OK,
            serde_json::json!({"call_id": id, "status": "resumed"}),
        ))
    })())
}

async fn transfer(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<Body>,
) -> Response {
    answer((|| {
        let (call_id, handler) = call(&state, &id, &body)?;
        let party = party(&body)?;
        let target = body
            .target
            .as_deref()
            .ok_or_else(|| CallControlError::bad_request("a transfer needs a `target`"))?;
        handler.transfer_call(&call_id, party, target)?;
        Ok((
            StatusCode::ACCEPTED,
            serde_json::json!({
                "call_id": id, "status": "transferring",
                "party": party_name(party), "target": target,
            }),
        ))
    })())
}

async fn redirect(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<Body>,
) -> Response {
    let outcome: Outcome = async {
        let (call_id, handler) = call(&state, &id, &body)?;
        let party = party(&body)?;
        let flow_id = body
            .flow_id
            .as_deref()
            .ok_or_else(|| CallControlError::bad_request("a redirect needs a `flow_id`"))?;
        handler.redirect_call(&call_id, party, flow_id).await?;
        Ok((
            StatusCode::ACCEPTED,
            serde_json::json!({
                "call_id": id, "status": "redirecting",
                "party": party_name(party), "flow_id": flow_id,
            }),
        ))
    }
    .await;
    answer(outcome)
}

async fn play(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<Body>,
) -> Response {
    let outcome: Outcome = async {
        let (call_id, handler) = call(&state, &id, &body)?;
        let to = audience(&body)?;
        let prompt_id = body
            .prompt_id
            .as_deref()
            .ok_or_else(|| CallControlError::bad_request("play needs a `prompt_id`"))?;
        let tenant = body.tenant_id.unwrap_or(state.default_tenant_id);
        let file = prompt_file(&state, prompt_id, tenant).await?;
        let playback_id = handler.play_into_call(&call_id, file, to).await?;
        Ok((
            StatusCode::OK,
            serde_json::json!({"call_id": id, "status": "playing", "playback_id": playback_id}),
        ))
    }
    .await;
    answer(outcome)
}

async fn say(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<Body>,
) -> Response {
    let outcome: Outcome = async {
        let (call_id, handler) = call(&state, &id, &body)?;
        let to = audience(&body)?;
        let text = body
            .text
            .as_deref()
            .filter(|t| !t.trim().is_empty())
            .ok_or_else(|| CallControlError::bad_request("say needs `text`"))?;
        let playback_id = handler
            .say_into_call(
                &call_id,
                text,
                body.language.as_deref(),
                body.voice.as_deref(),
                to,
            )
            .await?;
        Ok((
            StatusCode::OK,
            serde_json::json!({"call_id": id, "status": "playing", "playback_id": playback_id}),
        ))
    }
    .await;
    answer(outcome)
}

async fn hangup(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    body: Option<Json<Body>>,
) -> Response {
    let body = body.map(|Json(b)| b).unwrap_or_default();
    answer((|| {
        let (call_id, handler) = call(&state, &id, &body)?;
        handler
            .hangup_call_by_id_with(
                &call_id,
                fcp_sip::TerminationReason::system(
                    fcp_sip::SystemCause::Admin,
                    "Hung up via the API",
                ),
            )
            .map_err(|_| CallControlError::not_found(&id))?;
        Ok((
            StatusCode::OK,
            serde_json::json!({"call_id": id, "status": "ended"}),
        ))
    })())
}

fn party_name(party: fcp_b2bua_core::LegSide) -> &'static str {
    match party {
        fcp_b2bua_core::LegSide::A => "caller",
        fcp_b2bua_core::LegSide::B => "callee",
    }
}

/// The audio of the tenant's prompt `id`: another tenant's is not found.
async fn prompt_file(
    state: &AppState,
    id: &str,
    tenant: TenantId,
) -> Result<String, CallControlError> {
    let not_found = || CallControlError::new(404, format!("Prompt not found: {id}"));
    let prompt_id = id.parse::<fcp_core::PromptId>().map_err(|_| not_found())?;
    let store = state
        .storage
        .prompt_store()
        .ok_or_else(|| CallControlError::new(503, "prompts are not available on this node"))?;
    let prompt = store
        .get_prompt(&prompt_id)
        .await
        .map_err(|e| CallControlError::new(500, format!("reading the prompt: {e}")))?
        .filter(|p| p.tenant_id == tenant)
        .ok_or_else(not_found)?;
    prompt
        .audio_path
        .filter(|path| path.exists())
        .map(|path| path.to_string_lossy().into_owned())
        .ok_or_else(|| CallControlError::conflict(format!("the prompt {id} has no audio")))
}
