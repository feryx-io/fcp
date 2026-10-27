//! Signing in an app: OAuth 2.0 for native apps (RFC 8252) with PKCE
//! (RFC 7636), and app sessions (`docs/SOFTPHONE.md` §2.2, §2.3).
//!
//! An app opens the system browser at `GET /auth/app/authorize` with an
//! S256 challenge and a loopback `redirect_uri`. The browser lands on the
//! admin UI's sign-in (password, TOTP, the tenant's SSO — unchanged), which
//! then asks `POST /auth/app/approve` for the redirect back to the app with
//! a one-time code. The app trades the code and its verifier at
//! `POST /auth/app/token` for an access token and a refresh token, and
//! refreshes there.
//!
//! An app without a browser (`anvil-cli`) signs in as the UI does
//! (`/auth/login`, then `/auth/login/totp`) and asks
//! `POST /auth/app/sessions` with that fresh session's token: one password,
//! TOTP and lockout path for both.
//!
//! An app session is a `login_sessions` row naming the app: its refresh
//! token is opaque, rotated on every use (a token used twice ends the
//! session), and it lasts while it is used — the tenant's
//! `app_session_days` (90) from its last refresh. A second sign-in from the
//! same install replaces the first.

use crate::{
    error::ApiError,
    state::ApiState,
    v1::auth_session::{
        access_token, auth_user, publish, session_store, shown_role, AuthUserResponse, Me,
    },
};
use axum::{
    body::Bytes,
    extract::{Query, State},
    http::{header, HeaderMap},
    response::{IntoResponse, Redirect, Response},
    Json,
};
use fcp_core::session::{
    self, AppAuthorization, AppClient, LoginSession, LoginSessionStore, RefreshOutcome,
};
use fcp_core::{CredentialStore, LoginSessionId, TenantStore, User};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::SystemTime;

fn store(state: &ApiState) -> Result<Arc<dyn LoginSessionStore>, ApiError> {
    session_store(state)
        .ok_or_else(|| ApiError::ServiceUnavailable("App sign-in needs a database".to_string()))
}

fn storage(state: &ApiState) -> Result<&Arc<fcp_core::DynStorage>, ApiError> {
    state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::Internal("Storage not configured".to_string()))
}

/// What `GET /auth/app/authorize` takes.
#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct AuthorizeQuery {
    /// The app (`anvil`).
    pub client_id: String,
    /// Where the browser is sent back: a loopback address (RFC 8252 §7.3).
    pub redirect_uri: String,
    /// The S256 challenge of the app's verifier.
    pub code_challenge: String,
    /// Only `S256`.
    pub code_challenge_method: String,
    /// Returned to the app unchanged.
    pub state: Option<String>,
}

/// GET /api/v1/auth/app/authorize — an app begins its sign-in in the
/// browser, which is sent to the admin UI's sign-in to finish it.
#[utoipa::path(
    get,
    path = "/api/v1/auth/app/authorize",
    tag = "app_auth",
    operation_id = "app_auth_authorize",
    params(AuthorizeQuery),
    responses(
        (status = 303, description = "To the sign-in page, with `app` naming the request"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn authorize(
    State(state): State<ApiState>,
    Query(q): Query<AuthorizeQuery>,
) -> Result<Response, ApiError> {
    let bad = |why: &str| ApiError::BadRequest(why.to_string());
    if q.client_id.is_empty()
        || q.client_id.len() > 64
        || !q
            .client_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err(bad("client_id must be a short name"));
    }
    if !session::valid_app_redirect(&q.redirect_uri) || q.redirect_uri.len() > 512 {
        return Err(bad(
            "redirect_uri must be a loopback address: http://127.0.0.1, http://[::1] or http://localhost, any port",
        ));
    }
    if q.code_challenge_method != "S256" {
        return Err(bad("code_challenge_method must be S256"));
    }
    if q.code_challenge.len() != 43 {
        return Err(bad("code_challenge must be an S256 challenge"));
    }
    if q.state.as_ref().is_some_and(|s| s.len() > 512) {
        return Err(bad("state is too long"));
    }
    let now = SystemTime::now();
    let (id, _) = session::new_refresh_secret();
    store(&state)?
        .create_app_authorization(&AppAuthorization {
            id: id.clone(),
            client_id: q.client_id,
            redirect_uri: q.redirect_uri,
            code_challenge: q.code_challenge,
            state: q.state,
            created_at: now,
            expires_at: now + session::APP_AUTHORIZATION_TTL,
            user_id: None,
            tenant_id: None,
            code_hash: None,
            code_expires_at: None,
        })
        .await?;
    let base = state
        .sso
        .as_ref()
        .map(|s| s.public_url.clone())
        .unwrap_or_default();
    Ok(Redirect::to(&format!("{base}/login?app={}", urlencoding::encode(&id))).into_response())
}

/// What `POST /auth/app/approve` takes.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ApproveRequest {
    /// The `app` the sign-in page was opened with.
    pub request: String,
}

/// Where to send the browser.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ApproveResponse {
    /// The app's loopback address with `code` and `state`.
    pub redirect_url: String,
    /// The app that asked.
    pub client_id: String,
}

/// The caller's own browser session, signed in within the last ten
/// minutes: what may grant an app a session.
async fn fresh_session(state: &ApiState, me: &Me) -> Result<(User, LoginSession), ApiError> {
    let again = || ApiError::Unauthorized("Sign in again to sign in an app".to_string());
    if me.0.personal {
        return Err(ApiError::Forbidden(
            "An API token cannot sign in an app".to_string(),
        ));
    }
    let sid: LoginSessionId =
        me.0.sid
            .as_deref()
            .and_then(|s| s.parse().ok())
            .ok_or_else(again)?;
    let now = SystemTime::now();
    let row = store(state)?
        .get_login_session(&sid)
        .await?
        .filter(|s| s.is_live(now) && s.app.is_none())
        .ok_or_else(again)?;
    let signed_in_for = now.duration_since(row.created_at).unwrap_or_default();
    if signed_in_for > session::APP_GRANT_FRESHNESS {
        return Err(again());
    }
    let user = storage(state)?
        .get_user_by_id(&row.user_id)
        .await?
        .filter(|u| u.tenant_id == row.tenant_id && session::user_is_active(u))
        .ok_or_else(again)?;
    Ok((user, row))
}

/// POST /api/v1/auth/app/approve — the signed-in user grants the app's
/// sign-in: the address to send the browser to, with a one-time code.
#[utoipa::path(
    post,
    path = "/api/v1/auth/app/approve",
    tag = "app_auth",
    operation_id = "app_auth_approve",
    request_body = ApproveRequest,
    responses(
        (status = 200, description = "Where to send the browser", body = ApproveResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
    security(("bearer" = [])),
)]
pub async fn approve(
    State(state): State<ApiState>,
    me: Me,
    Json(req): Json<ApproveRequest>,
) -> Result<Json<ApproveResponse>, ApiError> {
    let (user, _) = fresh_session(&state, &me).await?;
    let store = store(&state)?;
    let gone =
        || ApiError::NotFound("The app's sign-in has ended; start again in the app".to_string());
    let now = SystemTime::now();
    let authorization = store
        .get_app_authorization(&req.request)
        .await?
        .filter(|a| a.expires_at > now && a.user_id.is_none())
        .ok_or_else(gone)?;
    let (code, code_hash) = session::new_refresh_secret();
    if !store
        .grant_app_authorization(
            &authorization.id,
            &user.id,
            &user.tenant_id,
            &code_hash,
            now + session::APP_CODE_TTL,
            now,
        )
        .await?
    {
        return Err(gone());
    }
    let mut url = authorization.redirect_uri.clone();
    url.push(if url.contains('?') { '&' } else { '?' });
    url.push_str(&format!("code={}", urlencoding::encode(&code)));
    if let Some(app_state) = &authorization.state {
        url.push_str(&format!("&state={}", urlencoding::encode(app_state)));
    }
    tracing::info!(username = %user.username, client_id = %authorization.client_id, "An app's sign-in granted");
    Ok(Json(ApproveResponse {
        redirect_url: url,
        client_id: authorization.client_id,
    }))
}

/// The app, as it names itself.
#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct AppClientRequest {
    /// What the app calls itself on this machine ("Alice's MacBook").
    pub client_name: String,
    /// Kept by the install across sign-ins.
    pub install_id: String,
    pub platform: Option<String>,
    pub version: Option<String>,
}

impl AppClientRequest {
    fn client(&self) -> Result<AppClient, ApiError> {
        let short = |v: &str, max: usize| !v.trim().is_empty() && v.len() <= max;
        if !short(&self.client_name, 128) || !short(&self.install_id, 128) {
            return Err(ApiError::BadRequest(
                "client_name and install_id are required, up to 128 characters".to_string(),
            ));
        }
        Ok(AppClient {
            name: self.client_name.trim().to_string(),
            install_id: self.install_id.trim().to_string(),
            platform: self.platform.clone().filter(|p| short(p, 64)),
            version: self.version.clone().filter(|v| short(v, 64)),
        })
    }
}

/// The install's device and its SIP account: the password is shown here
/// once, at each sign-in, and never again.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct AppDeviceResponse {
    pub id: String,
    pub name: String,
    pub sip_username: String,
    pub sip_password: String,
    /// The realm the password is hashed in, as the registrars challenge.
    pub realm: String,
}

/// An app session's tokens.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct AppTokenResponse {
    pub access_token: String,
    /// `Bearer`.
    pub token_type: &'static str,
    /// Seconds the access token lasts.
    pub expires_in: u64,
    /// Opaque; rotated on every refresh, so keep the newest.
    pub refresh_token: String,
    /// Seconds the session lasts unless refreshed.
    pub refresh_expires_in: u64,
    /// The session, as `/me/sessions` and the audit log name it.
    pub session_id: String,
    pub user: AuthUserResponse,
    /// The install's device, at a sign-in; absent from a refresh.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<AppDeviceResponse>,
}

/// Begin an app session for `user`: the install's earlier one ends.
async fn begin_app_session(
    state: &ApiState,
    user: &User,
    client: AppClient,
    headers: &HeaderMap,
) -> Result<AppTokenResponse, ApiError> {
    let store = store(state)?;
    let now = SystemTime::now();
    for earlier in store
        .app_sessions_of_install(&user.id, &client.install_id)
        .await?
    {
        store
            .revoke_login_session(&earlier.id, "the app signed in again", now)
            .await?;
    }
    let ttl = app_session_ttl(state, user).await?;
    let (device, sip_password) = install_device(state, user, &client).await?;
    let (secret, refresh_hash) = session::new_refresh_secret();
    let row = LoginSession {
        id: LoginSessionId::new(),
        tenant_id: user.tenant_id,
        user_id: user.id,
        method: session::method::APP.to_string(),
        created_at: now,
        refreshed_at: now,
        expires_at: now + ttl,
        revoked_at: None,
        revoked_reason: None,
        ip: crate::audit::source_ip(headers),
        user_agent: headers
            .get(header::USER_AGENT)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.chars().take(512).collect()),
        refresh_hash,
        previous_refresh_hash: None,
        idp: None,
        app: Some(client.clone()),
        device_id: Some(device.id),
    };
    store.create_login_session(&row).await?;
    publish(
        state,
        fcp_events::UserEvent::SignedIn {
            user_id: user.id.to_string(),
            username: user.username.clone(),
            tenant_id: user.tenant_id,
            method: session::method::APP.to_string(),
            session_id: row.id.to_string(),
            source_ip: crate::audit::source_ip(headers),
            timestamp: now,
        },
    );
    tracing::info!(username = %user.username, app = %client.name, install = %client.install_id, "An app signed in");
    let mut issued = tokens(state, user, &row.id, &secret, ttl, now).await?;
    issued.device = Some(AppDeviceResponse {
        id: device.id.to_string(),
        name: device.name.clone(),
        sip_username: device.sip_username.clone().unwrap_or_default(),
        sip_password,
        realm: state.realm.clone(),
    });
    Ok(issued)
}

/// The install's device (`docs/SOFTPHONE.md` §2.1): found by the install,
/// or made — a softphone owned by the user, with a SIP username of its own —
/// and given a new SIP password, which only this sign-in learns.
async fn install_device(
    state: &ApiState,
    user: &User,
    client: &AppClient,
) -> Result<(fcp_core::Device, String), ApiError> {
    use fcp_core::storage::DeviceStore;
    let storage = storage(state)?;
    let sessions = store(state)?;
    let known = match sessions
        .app_install_device(&user.id, &client.install_id)
        .await?
    {
        Some(id) => storage
            .get_device(&id)
            .await?
            .filter(|d| d.owner_user_id == Some(user.id) && d.tenant_id == user.tenant_id),
        None => None,
    };
    let mut device = match known {
        Some(device) => device,
        None => {
            let mut device = fcp_core::Device::new(
                client.name.clone(),
                fcp_core::DeviceType::Softphone,
                user.tenant_id,
            );
            device.owner_user_id = Some(user.id);
            device.sip_username = Some(free_sip_username(storage, &user.tenant_id).await?);
            device
        }
    };
    device.name = client.name.clone();
    let password = set_sip_password(state, &mut device).await?;
    sessions
        .set_app_install_device(&user.tenant_id, &user.id, &client.install_id, &device.id)
        .await?;
    Ok((device, password))
}

/// Give an app's device a new SIP password, keep only its digest hash in
/// the realm the registrars challenge in, and save the device: the
/// password, which nothing else will learn.
pub(crate) async fn set_sip_password(
    state: &ApiState,
    device: &mut fcp_core::Device,
) -> Result<String, ApiError> {
    use fcp_core::storage::DeviceStore;
    let (password, _) = session::new_refresh_secret();
    let password: String = password.chars().take(24).collect();
    let username = device.sip_username.clone().unwrap_or_default();
    device.sip_ha1_hash = Some(fcp_core::storage::password::compute_ha1(
        &username,
        &state.realm,
        &password,
    ));
    device.sip_provisioning_password = None;
    device.updated_at = SystemTime::now();
    storage(state)?.put_device(device.clone()).await?;
    let _ = state
        .event_bus
        .publish(fcp_events::PlatformEvent::System(
            fcp_events::SystemEvent::ConfigChanged {
                entity_type: "device".to_string(),
                entity_id: device.id.to_string(),
                tenant_id: device.tenant_id,
                timestamp: SystemTime::now(),
            },
        ))
        .await;
    Ok(password)
}

/// A SIP username for a new app device: `app-` and ten hex digits, held by
/// no user and no device of the tenant.
async fn free_sip_username(
    storage: &fcp_core::DynStorage,
    tenant: &fcp_core::TenantId,
) -> Result<String, ApiError> {
    use fcp_core::storage::DeviceStore;
    for _ in 0..8 {
        let name = format!("app-{}", &uuid::Uuid::new_v4().simple().to_string()[..10]);
        let taken = storage.get_user_by_sip_name(&name, tenant).await?.is_some()
            || storage
                .get_device_by_sip_name(tenant, &name)
                .await?
                .is_some();
        if !taken {
            return Ok(name);
        }
    }
    Err(ApiError::Internal(
        "No free SIP username for the app".to_string(),
    ))
}

async fn app_session_ttl(state: &ApiState, user: &User) -> Result<std::time::Duration, ApiError> {
    Ok(storage(state)?
        .get_tenant(&user.tenant_id)
        .await?
        .map(|t| t.sign_in.app_session_ttl())
        .unwrap_or_else(|| fcp_core::TenantSignIn::default().app_session_ttl()))
}

async fn tokens(
    state: &ApiState,
    user: &User,
    sid: &LoginSessionId,
    secret: &str,
    ttl: std::time::Duration,
    now: SystemTime,
) -> Result<AppTokenResponse, ApiError> {
    let (access, expires_in) = access_token(state, user, Some(sid), now)?;
    Ok(AppTokenResponse {
        access_token: access,
        token_type: "Bearer",
        expires_in,
        refresh_token: session::refresh_cookie_value(sid, secret),
        refresh_expires_in: ttl.as_secs(),
        session_id: sid.to_string(),
        user: auth_user(user, shown_role(state, user).await),
        device: None,
    })
}

/// POST /api/v1/auth/app/sessions — an app without a browser
/// (`anvil-cli`) gets a session of its own from a session signed in within
/// the last ten minutes (`/auth/login`, `/auth/login/totp`).
#[utoipa::path(
    post,
    path = "/api/v1/auth/app/sessions",
    tag = "app_auth",
    operation_id = "app_auth_create_session",
    request_body = AppClientRequest,
    responses(
        (status = 201, description = "The app's session", body = AppTokenResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
    security(("bearer" = [])),
)]
pub async fn create_session(
    State(state): State<ApiState>,
    headers: HeaderMap,
    me: Me,
    Json(req): Json<AppClientRequest>,
) -> Result<Response, ApiError> {
    let client = req.client()?;
    let (user, _) = fresh_session(&state, &me).await?;
    let issued = begin_app_session(&state, &user, client, &headers).await?;
    Ok((axum::http::StatusCode::CREATED, Json(issued)).into_response())
}

/// What `POST /auth/app/token` takes, as JSON or a form (RFC 6749 §4.1.3,
/// §6).
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct AppTokenRequest {
    /// `authorization_code` or `refresh_token`.
    pub grant_type: String,
    pub code: Option<String>,
    pub code_verifier: Option<String>,
    pub redirect_uri: Option<String>,
    pub client_id: Option<String>,
    pub refresh_token: Option<String>,
    /// With `authorization_code`: the app, as `/auth/app/sessions` takes it.
    pub client_name: Option<String>,
    pub install_id: Option<String>,
    pub platform: Option<String>,
    pub version: Option<String>,
}

fn parse_body<T: serde::de::DeserializeOwned>(
    headers: &HeaderMap,
    body: &Bytes,
) -> Result<T, ApiError> {
    let form = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("application/x-www-form-urlencoded"));
    if form {
        serde_urlencoded::from_bytes(body)
            .map_err(|e| ApiError::BadRequest(format!("Invalid form: {e}")))
    } else {
        serde_json::from_slice(body).map_err(|e| ApiError::BadRequest(format!("Invalid JSON: {e}")))
    }
}

/// POST /api/v1/auth/app/token — a code and its verifier for an app
/// session, or a refresh token for the next one.
#[utoipa::path(
    post,
    path = "/api/v1/auth/app/token",
    tag = "app_auth",
    operation_id = "app_auth_token",
    request_body(content = AppTokenRequest, content_type = "application/json"),
    responses(
        (status = 200, description = "The app session's tokens", body = AppTokenResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn token(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<AppTokenResponse>, ApiError> {
    let req: AppTokenRequest = parse_body(&headers, &body)?;
    match req.grant_type.as_str() {
        "authorization_code" => redeem(&state, &headers, req).await.map(Json),
        "refresh_token" => refresh(&state, req).await.map(Json),
        other => Err(ApiError::BadRequest(format!(
            "grant_type {other:?} is not taken: authorization_code or refresh_token"
        ))),
    }
}

async fn redeem(
    state: &ApiState,
    headers: &HeaderMap,
    req: AppTokenRequest,
) -> Result<AppTokenResponse, ApiError> {
    let refused = || ApiError::Unauthorized("The code is not valid; sign in again".to_string());
    let client = AppClientRequest {
        client_name: req.client_name.clone().unwrap_or_default(),
        install_id: req.install_id.clone().unwrap_or_default(),
        platform: req.platform.clone(),
        version: req.version.clone(),
    }
    .client()?;
    let (code, verifier) = match (req.code.as_deref(), req.code_verifier.as_deref()) {
        (Some(c), Some(v)) if session::valid_pkce_verifier(v) => (c, v),
        _ => {
            return Err(ApiError::BadRequest(
                "code and a code_verifier (43 to 128 characters) are required".to_string(),
            ))
        }
    };
    let now = SystemTime::now();
    // Redeemed whatever follows: a code is tried once.
    let authorization = store(state)?
        .redeem_app_code(&session::refresh_hash(code), now)
        .await?
        .ok_or_else(refused)?;
    if req.redirect_uri.as_deref() != Some(authorization.redirect_uri.as_str())
        || req
            .client_id
            .as_deref()
            .is_some_and(|c| c != authorization.client_id)
        || session::pkce_challenge(verifier) != authorization.code_challenge
    {
        return Err(refused());
    }
    let (Some(user_id), Some(tenant_id)) = (authorization.user_id, authorization.tenant_id) else {
        return Err(refused());
    };
    let user = storage(state)?
        .get_user_by_id(&user_id)
        .await?
        .filter(|u| u.tenant_id == tenant_id && session::user_is_active(u))
        .ok_or_else(refused)?;
    begin_app_session(state, &user, client, headers).await
}

async fn refresh(state: &ApiState, req: AppTokenRequest) -> Result<AppTokenResponse, ApiError> {
    let ended = || ApiError::Unauthorized("The app's session has ended; sign in again".to_string());
    let (id, secret) = req
        .refresh_token
        .as_deref()
        .and_then(session::parse_refresh_cookie)
        .ok_or_else(ended)?;
    let store = store(state)?;
    let now = SystemTime::now();
    // An app's token, never a browser's: a browser's cookie is not turned
    // over here.
    store
        .get_login_session(&id)
        .await?
        .filter(|s| s.app.is_some() && s.is_live(now))
        .ok_or_else(ended)?;
    let (next_secret, next_hash) = session::new_refresh_secret();
    match store
        .rotate_refresh(&id, &session::refresh_hash(&secret), &next_hash, now)
        .await?
    {
        RefreshOutcome::Rotated => {}
        RefreshOutcome::RecentlyRotated => {
            return Err(ApiError::Conflict(
                "That refresh token was just replaced: use the newest".to_string(),
            ))
        }
        RefreshOutcome::Reused => {
            tracing::warn!(session = %id, "An app's refresh token was used twice: the session is revoked");
            return Err(ended());
        }
        RefreshOutcome::Unknown => return Err(ended()),
    }
    let row = store
        .get_login_session(&id)
        .await?
        .filter(|s| s.is_live(now))
        .ok_or_else(ended)?;
    let user = storage(state)?
        .get_user_by_id(&row.user_id)
        .await?
        .filter(|u| u.tenant_id == row.tenant_id && session::user_is_active(u))
        .ok_or_else(ended)?;
    // It lasts while it is used.
    let ttl = app_session_ttl(state, &user).await?;
    store.extend_login_session(&id, now + ttl).await?;
    tokens(state, &user, &id, &next_secret, ttl, now).await
}

/// What `POST /auth/app/revoke` takes (RFC 7009).
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct AppRevokeRequest {
    pub token: String,
}

/// POST /api/v1/auth/app/revoke — an app signs out: its session ends and
/// the install's device is removed. Answers 200 whether or not the token named a live session (RFC 7009
/// §2.2).
#[utoipa::path(
    post,
    path = "/api/v1/auth/app/revoke",
    tag = "app_auth",
    operation_id = "app_auth_revoke",
    request_body(content = AppRevokeRequest, content_type = "application/json"),
    responses(
        (status = 200, description = "The session, if any, has ended"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn revoke(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<axum::http::StatusCode, ApiError> {
    let req: AppRevokeRequest = parse_body(&headers, &body)?;
    let Some((id, secret)) = session::parse_refresh_cookie(&req.token) else {
        return Ok(axum::http::StatusCode::OK);
    };
    let store = store(&state)?;
    let hash = session::refresh_hash(&secret);
    let now = SystemTime::now();
    if let Some(row) = store.get_login_session(&id).await? {
        if row.app.is_some() && row.refresh_hash == hash {
            store
                .revoke_login_session(&id, "the app signed out", now)
                .await?;
            // The install's device goes with it: its registrations
            // dropped, its SIP account gone, off the Devices page. A
            // signed-out app stayed there with its bindings
            // (`docs/DEEP_TESTING.md` D8). Signing in again makes another.
            if let Some(device_id) = row.device_id {
                use fcp_core::storage::DeviceStore;
                let storage = storage(&state)?;
                if let Some(device) = storage.get_device(&device_id).await? {
                    crate::v1::devices::remove_device(&state, storage, &device).await?;
                    tracing::info!(device = %device.id, "An app signed out; its device removed");
                }
            }
        }
    }
    Ok(axum::http::StatusCode::OK)
}

/// How long an app device may go without a session before the sweep
/// removes it: long enough that a device being signed in, made a moment
/// before its session, is never taken.
const ABANDONED_AFTER: std::time::Duration = std::time::Duration::from_secs(3600);

/// Remove the app devices no live session uses (`docs/APP_SIGN_IN.md`):
/// an install that signed out before sign-out removed its device, whose
/// session expired, or that was replaced without signing out. Its
/// registrations go with it. How many were removed.
pub async fn sweep_abandoned_devices(state: &ApiState, now: SystemTime) -> u64 {
    use fcp_core::storage::DeviceStore;
    let (Ok(sessions), Ok(storage)) = (store(state), storage(state)) else {
        return 0;
    };
    let devices = match sessions.abandoned_app_devices(now - ABANDONED_AFTER).await {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!(error = %e, "Abandoned app devices not listed");
            return 0;
        }
    };
    let mut removed = 0;
    for id in devices {
        match storage.get_device(&id).await {
            Ok(Some(device)) => {
                match crate::v1::devices::remove_device(state, storage, &device).await {
                    Ok(()) => removed += 1,
                    Err(e) => {
                        tracing::warn!(device = %id, error = ?e, "Abandoned app device not removed")
                    }
                }
            }
            Ok(None) => {}
            Err(e) => tracing::warn!(device = %id, error = %e, "Abandoned app device not read"),
        }
    }
    removed
}

/// Run [`sweep_abandoned_devices`] once an hour, one admin replica at a time.
pub fn spawn_device_sweep(state: ApiState) {
    use fcp_core::storage::CdrStore;
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));
        loop {
            interval.tick().await;
            let Some(storage) = state.storage.clone() else {
                continue;
            };
            let lock = match storage.try_maintenance_lock("fcp:app-devices").await {
                Ok(Some(lock)) => lock,
                Ok(None) => continue,
                Err(e) => {
                    tracing::warn!(error = %e, "App device sweep: could not take the lock");
                    continue;
                }
            };
            let removed = sweep_abandoned_devices(&state, SystemTime::now()).await;
            if removed > 0 {
                tracing::info!(removed, "Abandoned app devices removed");
            }
            lock.release().await;
        }
    });
}
