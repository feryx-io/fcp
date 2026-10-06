//! Signing in (`docs/IDENTITY_AND_DEVICES.md` phase 1).
//!
//! A sign-in makes a session (a row: `fcp_core::session`), answers a short
//! access token naming it and the user's id, and sets the session's refresh
//! secret in an HttpOnly cookie scoped to `/api/v1/auth`:
//! - POST /api/v1/auth/login   — a password; the access token and the cookie
//! - POST /api/v1/auth/refresh — the cookie; a new access token, the secret
//!   turned over
//! - POST /api/v1/auth/logout  — the session revoked, the cookie cleared
//! - GET  /api/v1/auth/me      — who the access token is
//! - POST /api/v1/auth/change-expired-password — change an expired password

use crate::error::ApiError;
use crate::state::ApiState;
use axum::{
    extract::State,
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use fcp_core::session::{self, LoginSessionStore, RefreshOutcome, SessionClaims};
use fcp_core::storage::password;
use fcp_core::LoginSessionId;
use fcp_core::{AccountStatus, CredentialStore, TenantStore};
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime};

use fcp_core::UserRole;

/// How long a token made without a session store lasts, as every token
/// did before sessions were rows.
const LEGACY_TOKEN_HOURS: u64 = 24;

/// The cookie a session's refresh secret travels in.
pub const REFRESH_COOKIE: &str = "fcp_refresh";
/// Where the browser sends it: the sign-in routes, and nowhere else.
const REFRESH_COOKIE_PATH: &str = "/api/v1/auth";

/// Serialize a UserRole to its serde snake_case string (e.g. EndUser -> "end_user").
pub fn role_to_string(role: &UserRole) -> String {
    serde_json::to_value(role)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_else(|| format!("{:?}", role).to_lowercase())
}

/// A user's roles now: their own and their groups' (`docs/USERS.md` §5).
pub(crate) async fn roles_of(state: &ApiState, user: &fcp_core::User) -> Vec<UserRole> {
    let mut roles = vec![user.role];
    if let Some(storage) = state.storage.as_ref() {
        match storage.group_roles_of_user(&user.id).await {
            Ok(more) => roles.extend(more),
            Err(e) => {
                tracing::warn!(user_id = %user.id, error = %e, "Could not read a user's groups")
            }
        }
    }
    roles
}

/// The role to show for `user`: the most privileged they hold.
pub(crate) async fn shown_role(state: &ApiState, user: &fcp_core::User) -> String {
    display_role(&roles_of(state, user).await)
}

/// The role to show for someone holding `roles` (their own and their
/// groups'): the most privileged. Decisions ask each role instead
/// (`fcp_core::permissions::roles_have_permission`).
pub fn display_role(roles: &[UserRole]) -> String {
    role_to_string(&UserRole::most_privileged(roles.iter().copied()).unwrap_or_default())
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub tenant_id: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct LoginResponse {
    /// The access token: `Authorization: Bearer <token>`. Absent while a
    /// second factor is wanted (`mfa_required`) or the password must be
    /// changed (`password_expired`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// Seconds the access token lasts; refresh before then.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_in: Option<u64>,
    pub user: AuthUserResponse,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password_expired: Option<bool>,
    /// The password was right; now a second factor
    /// (`docs/IDENTITY_AND_DEVICES.md` phase 2b): `totp` — a code to
    /// `/auth/login/totp` — or `totp_enrollment` — the tenant requires one
    /// the user has not set up: `/auth/totp/enroll` then `/auth/totp/confirm`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mfa_required: Option<String>,
    /// The ticket those routes take, valid five minutes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mfa_token: Option<String>,
    /// Recovery codes, shown once, when a second factor was just set up.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recovery_codes: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Clone, utoipa::ToSchema)]
pub struct AuthUserResponse {
    pub username: String,
    pub role: String,
    pub tenant_id: String,
    pub display_name: Option<String>,
}

/// The caller of a `/me` route, as its handler reads it ([`Me`]): the
/// username as it is now, the role, the tenant, and the user's id and
/// session where known.
#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Claims {
    /// The username, as it is now.
    pub sub: String,
    pub role: String,
    pub tenant_id: String,
    pub display_name: Option<String>,
    pub exp: usize,
    pub iat: usize,
    /// The user's id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    /// The sign-in session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sid: Option<String>,
    /// The caller came with a personal API token, not a session.
    #[serde(skip)]
    pub personal: bool,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ChangeExpiredPasswordRequest {
    pub username: String,
    pub current_password: String,
    pub new_password: String,
    #[serde(default)]
    pub tenant_id: Option<String>,
}

fn parse_tenant_id(tenant_id: Option<&str>) -> Result<Option<fcp_core::TenantId>, ApiError> {
    tenant_id
        .filter(|tenant_id| !tenant_id.trim().is_empty())
        .map(|tenant_id| {
            tenant_id
                .parse::<fcp_core::TenantId>()
                .map_err(|e| ApiError::BadRequest(format!("Invalid tenant_id: {}", e)))
        })
        .transpose()
}

/// Who `login` is: a username, or an email address in the tenant its domain
/// belongs to (`docs/IDENTITY_AND_DEVICES.md` phase 2), matched first as a
/// username and then by the users' email. An email several users share
/// names nobody: the caller is told no more than for a wrong password.
async fn load_user_for_login(
    storage: &std::sync::Arc<fcp_core::DynStorage>,
    login: &str,
    tenant_id: Option<&fcp_core::TenantId>,
) -> Result<Option<fcp_core::User>, ApiError> {
    let tenant = match tenant_id {
        Some(tenant) => Some(*tenant),
        None => match fcp_core::login_domain(login) {
            Some(domain) => storage
                .get_tenant_by_login_domain(&domain)
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?
                .map(|t| t.id),
            None => None,
        },
    };
    if let Some(tenant) = tenant {
        if let Some(user) = storage
            .get_user_in_tenant(login, &tenant)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
        {
            return Ok(Some(user));
        }
        if !login.contains('@') {
            return Ok(None);
        }
        let mut found = storage
            .find_users_by_email_in_tenant(login, &tenant)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        return Ok((found.len() == 1).then(|| found.remove(0)));
    }

    let users = storage
        .list_users_by_username(login)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    match users.len() {
        0 => Ok(None),
        1 => Ok(users.into_iter().next()),
        _ => Err(ApiError::BadRequest(
            "tenant_id is required for usernames that exist in multiple tenants".to_string(),
        )),
    }
}

/// What a login may sign in with.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct DiscoverRequest {
    /// A username, or an email address.
    pub login: String,
}

/// Where a login signs in, and how.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct DiscoverResponse {
    /// The tenant the login's email domain belongs to, when it is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tenant_name: Option<String>,
    /// The ways in: `password`, and `sso` when the tenant has identity
    /// providers to sign in with.
    pub methods: Vec<String>,
    /// The tenant's identity providers: a button each on the login page,
    /// which goes to `/api/v1/auth/sso/{id}/start`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub providers: Vec<DiscoveredProvider>,
}

/// An identity provider a login can sign in through.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct DiscoveredProvider {
    pub id: String,
    pub name: String,
}

/// POST /api/v1/auth/discover — where a login signs in: the tenant its
/// email domain belongs to, and the ways in. It never says whether a user
/// exists.
#[utoipa::path(
    post,
    path = "/api/v1/auth/discover",
    tag = "auth_session",
    operation_id = "auth_session_discover",
    request_body = DiscoverRequest,
    responses(
        (status = 200, description = "OK", body = DiscoverResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn discover(
    State(state): State<ApiState>,
    Json(req): Json<DiscoverRequest>,
) -> Result<Json<DiscoverResponse>, ApiError> {
    let storage = state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::Internal("Storage not configured".to_string()))?;
    let tenant = match fcp_core::login_domain(&req.login) {
        Some(domain) => storage
            .get_tenant_by_login_domain(&domain)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?,
        None => None,
    };
    let password = tenant
        .as_ref()
        .is_none_or(|t| t.sign_in.password_login != fcp_core::PasswordLogin::Disabled);
    let providers = match &tenant {
        Some(t) => crate::v1::sso::offered(&state, &t.id).await,
        None => Vec::new(),
    };
    let mut methods = Vec::new();
    if password {
        methods.push("password".to_string());
    }
    if !providers.is_empty() {
        methods.push("sso".to_string());
    }
    Ok(Json(DiscoverResponse {
        tenant_id: tenant.as_ref().map(|t| t.id.to_string()),
        tenant_name: tenant.map(|t| t.name),
        methods,
        providers: providers
            .into_iter()
            .map(|p| DiscoveredProvider {
                id: p.id.to_string(),
                name: p.name,
            })
            .collect(),
    }))
}

// ---- Sessions (docs/IDENTITY_AND_DEVICES.md phase 1) ----

/// What a sign-in hands the browser.
pub(crate) struct Issued {
    pub token: String,
    pub expires_in: u64,
    /// The refresh cookie, when a session store keeps sessions.
    pub cookie: Option<HeaderValue>,
    pub session: Option<LoginSessionId>,
}

pub(crate) fn session_store(state: &ApiState) -> Option<std::sync::Arc<dyn LoginSessionStore>> {
    state
        .storage
        .as_ref()
        .and_then(|s| s.login_session_store().cloned())
}

pub(crate) fn keys(state: &ApiState) -> Result<&session::SessionKeys, ApiError> {
    state
        .session_keys
        .as_ref()
        .ok_or_else(|| ApiError::Internal("Sign-in is not configured".to_string()))
}

/// An access token for `user` in `session`, and how long it lasts.
pub(crate) fn access_token(
    state: &ApiState,
    user: &fcp_core::User,
    sid: Option<&LoginSessionId>,
    now: SystemTime,
) -> Result<(String, u64), ApiError> {
    let ttl = match sid {
        Some(_) => state.session_access_ttl.as_secs(),
        None => LEGACY_TOKEN_HOURS * 3600,
    };
    let iat = session::epoch(now);
    let claims = SessionClaims {
        // A session's token names the user by id; one made without a
        // session store names the username, as tokens did before.
        sub: match sid {
            Some(_) => user.id.to_string(),
            None => user.username.clone(),
        },
        sid: sid.map(|s| s.to_string()),
        username: Some(user.username.clone()),
        role: role_to_string(&user.role),
        tenant_id: user.tenant_id.to_string(),
        display_name: user.display_name.clone(),
        exp: iat + ttl as usize,
        iat,
        purpose: None,
    };
    Ok((keys(state)?.encode(&claims), ttl))
}

/// Whether the request reached us over HTTPS, as the proxy in front says.
fn is_https(headers: &HeaderMap) -> bool {
    headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("https"))
}

fn cookie(value: &str, max_age: u64, secure: bool) -> HeaderValue {
    let secure = if secure { "; Secure" } else { "" };
    HeaderValue::from_str(&format!(
        "{REFRESH_COOKIE}={value}; Path={REFRESH_COOKIE_PATH}; Max-Age={max_age}; HttpOnly; SameSite=Strict{secure}"
    ))
    .expect("a cookie of base64url and ids is a header value")
}

/// The cookie that clears the refresh cookie.
fn clear_cookie(headers: &HeaderMap) -> HeaderValue {
    cookie("", 0, is_https(headers))
}

/// The refresh cookie the request carries.
fn refresh_cookie(headers: &HeaderMap) -> Option<(LoginSessionId, String)> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .find_map(|pair| {
            let (name, value) = pair.trim().split_once('=')?;
            (name == REFRESH_COOKIE).then(|| session::parse_refresh_cookie(value))?
        })
}

/// Begin a session for `user`, who has just proved who they are by
/// `method`: the session's row, its access token and its refresh cookie.
/// Without a session store, a token as before sessions were rows.
pub(crate) async fn begin_session(
    state: &ApiState,
    user: &fcp_core::User,
    method: &str,
    headers: &HeaderMap,
    idp: Option<session::IdpSession>,
) -> Result<Issued, ApiError> {
    let now = SystemTime::now();
    let Some(store) = session_store(state) else {
        let (token, expires_in) = access_token(state, user, None, now)?;
        return Ok(Issued {
            token,
            expires_in,
            cookie: None,
            session: None,
        });
    };
    let (secret, refresh_hash) = session::new_refresh_secret();
    let row = session::LoginSession {
        id: LoginSessionId::new(),
        tenant_id: user.tenant_id,
        user_id: user.id,
        method: method.to_string(),
        created_at: now,
        refreshed_at: now,
        expires_at: now + state.session_ttl,
        revoked_at: None,
        revoked_reason: None,
        ip: crate::audit::source_ip(headers),
        user_agent: headers
            .get(header::USER_AGENT)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.chars().take(512).collect()),
        refresh_hash,
        previous_refresh_hash: None,
        idp,
        app: None,
    };
    store
        .create_login_session(&row)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to start a session: {e}")))?;
    let (token, expires_in) = access_token(state, user, Some(&row.id), now)?;
    Ok(Issued {
        token,
        expires_in,
        cookie: Some(cookie(
            &session::refresh_cookie_value(&row.id, &secret),
            state.session_ttl.as_secs(),
            is_https(headers),
        )),
        session: Some(row.id),
    })
}

/// A sign-in's answer: the body, and the cookie when there is one.
fn signed_in(
    issued: Issued,
    user: &fcp_core::User,
    role: String,
    password_expired: bool,
) -> Response {
    let body = Json(LoginResponse {
        token: Some(issued.token),
        expires_in: Some(issued.expires_in),
        user: auth_user(user, role),
        password_expired: password_expired.then_some(true),
        mfa_required: None,
        mfa_token: None,
        recovery_codes: None,
    });
    let mut response = (StatusCode::OK, body).into_response();
    if let Some(cookie) = issued.cookie {
        response.headers_mut().append(header::SET_COOKIE, cookie);
    }
    response
}

pub(crate) fn auth_user(user: &fcp_core::User, role: String) -> AuthUserResponse {
    AuthUserResponse {
        username: user.username.clone(),
        role,
        tenant_id: user.tenant_id.to_string(),
        display_name: user.display_name.clone(),
    }
}

// ---- The second factor (docs/IDENTITY_AND_DEVICES.md phase 2b) ----

/// A ticket's purposes: a code, or setting one up.
const MFA_TOTP: &str = "totp";
const MFA_ENROLL: &str = "totp_enrollment";
/// How long a sign-in may wait between its password and its code.
const MFA_TICKET_SECS: u64 = 300;

/// The ticket a sign-in halfway through takes to its second step.
fn mfa_ticket(state: &ApiState, user: &fcp_core::User, purpose: &str) -> Result<String, ApiError> {
    let iat = session::epoch(SystemTime::now());
    Ok(keys(state)?.encode(&SessionClaims {
        sub: user.id.to_string(),
        sid: None,
        username: Some(user.username.clone()),
        role: role_to_string(&user.role),
        tenant_id: user.tenant_id.to_string(),
        display_name: None,
        exp: iat + MFA_TICKET_SECS as usize,
        iat,
        purpose: Some(purpose.to_string()),
    }))
}

/// The user a ticket for `purpose` names, still active.
async fn ticket_user(
    state: &ApiState,
    ticket: &str,
    purpose: &str,
) -> Result<fcp_core::User, ApiError> {
    let expired = || ApiError::Unauthorized("Sign in again".to_string());
    let claims = keys(state)?
        .decode(ticket, SystemTime::now())
        .map_err(|_| expired())?;
    if claims.purpose.as_deref() != Some(purpose) {
        return Err(expired());
    }
    let id: fcp_core::UserId = claims.sub.parse().map_err(|_| expired())?;
    let storage = state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::Internal("Storage not configured".to_string()))?;
    storage
        .get_user_by_id(&id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .filter(|u| u.tenant_id.to_string() == claims.tenant_id && session::user_is_active(u))
        .ok_or_else(expired)
}

fn totp_store(state: &ApiState) -> Result<std::sync::Arc<dyn fcp_core::totp::TotpStore>, ApiError> {
    state
        .storage
        .as_ref()
        .and_then(|s| s.totp_store().cloned())
        .ok_or_else(|| ApiError::Internal("Second factors are not available".to_string()))
}

/// A second step wanted: the ticket, and no session yet.
fn challenge(
    state: &ApiState,
    user: &fcp_core::User,
    role: String,
    purpose: &str,
) -> Result<Response, ApiError> {
    Ok(Json(LoginResponse {
        token: None,
        expires_in: Some(MFA_TICKET_SECS),
        user: auth_user(user, role),
        password_expired: None,
        mfa_required: Some(purpose.to_string()),
        mfa_token: Some(mfa_ticket(state, user, purpose)?),
        recovery_codes: None,
    })
    .into_response())
}

/// A password has proved who `user` is: a code next if they have a second
/// factor, setting one up if their tenant requires one they lack, else a
/// session.
async fn after_password(
    state: &ApiState,
    headers: &HeaderMap,
    user: fcp_core::User,
) -> Result<Response, ApiError> {
    if let Ok(store) = totp_store(state) {
        let enrolled = store
            .get_user_totp(&user.id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
            .is_some_and(|t| t.is_confirmed());
        if enrolled {
            return challenge(state, &user, shown_role(state, &user).await, MFA_TOTP);
        }
        let required = state
            .storage
            .as_ref()
            .ok_or_else(|| ApiError::Internal("Storage not configured".to_string()))?
            .get_tenant(&user.tenant_id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
            .map(|t| t.sign_in.require_totp);
        let roles = roles_of(state, &user).await;
        let required = required.is_some_and(|r| roles.iter().any(|role| r.applies(role)));
        if required {
            return challenge(state, &user, display_role(&roles), MFA_ENROLL);
        }
    }
    complete_sign_in(state, headers, user, session::method::PASSWORD, None).await
}

/// Signed in: the failed attempts forgotten, the session begun, published.
pub(crate) async fn complete_sign_in(
    state: &ApiState,
    headers: &HeaderMap,
    user: fcp_core::User,
    method: &str,
    recovery_codes: Option<Vec<String>>,
) -> Result<Response, ApiError> {
    complete_sign_in_through(state, headers, user, method, recovery_codes, None).await
}

/// Signed in, through an identity provider when `idp` says which: the
/// session remembers it, for the provider's back-channel logout.
pub(crate) async fn complete_sign_in_through(
    state: &ApiState,
    headers: &HeaderMap,
    mut user: fcp_core::User,
    method: &str,
    recovery_codes: Option<Vec<String>>,
    idp: Option<session::IdpSession>,
) -> Result<Response, ApiError> {
    let now = SystemTime::now();
    if let Some(storage) = state.storage.as_ref() {
        user.failed_login_attempts = 0;
        user.last_login = Some(now);
        user.updated_at = now;
        let _ = storage.put_user(user.clone()).await;
    }
    let issued = begin_session(state, &user, method, headers, idp).await?;
    if let Some(session_id) = &issued.session {
        publish(
            state,
            fcp_events::UserEvent::SignedIn {
                user_id: user.id.to_string(),
                username: user.username.clone(),
                tenant_id: user.tenant_id,
                method: method.to_string(),
                session_id: session_id.to_string(),
                source_ip: crate::audit::source_ip(headers),
                timestamp: now,
            },
        );
    }
    tracing::info!(username = %user.username, tenant_id = %user.tenant_id, method, "Login successful");
    let role = shown_role(state, &user).await;
    if recovery_codes.is_none() {
        return Ok(signed_in(issued, &user, role, false));
    }
    let mut response = Json(LoginResponse {
        token: Some(issued.token),
        expires_in: Some(issued.expires_in),
        user: auth_user(&user, role),
        password_expired: None,
        mfa_required: None,
        mfa_token: None,
        recovery_codes,
    })
    .into_response();
    if let Some(cookie) = issued.cookie {
        response.headers_mut().append(header::SET_COOKIE, cookie);
    }
    Ok(response)
}

/// A wrong second factor counts as a wrong password: the same lockout.
async fn count_failure(
    state: &ApiState,
    headers: &HeaderMap,
    mut user: fcp_core::User,
) -> ApiError {
    let now = SystemTime::now();
    let policy = &state.security_policy;
    user.failed_login_attempts += 1;
    user.updated_at = now;
    if user.failed_login_attempts >= policy.max_failed_attempts as i32 {
        user.account_status = AccountStatus::Locked;
        user.account_locked_until = Some(now + Duration::from_secs(policy.lockout_duration_secs));
        tracing::warn!(event = "account_locked", username = %user.username, "Account locked after too many wrong codes");
    }
    let (username, tenant) = (user.username.clone(), user.tenant_id);
    if let Some(storage) = state.storage.as_ref() {
        let _ = storage.put_user(user).await;
    }
    refused(
        state,
        headers,
        &username,
        Some(tenant),
        "bad_code",
        ApiError::Unauthorized("The code is not right".to_string()),
    )
}

/// The second step of a sign-in with a second factor.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct TotpLoginRequest {
    /// The ticket the password step answered.
    pub mfa_token: String,
    /// The app's six-digit code.
    #[serde(default)]
    pub code: Option<String>,
    /// Or one of the recovery codes, each good once.
    #[serde(default)]
    pub recovery_code: Option<String>,
}

/// POST /api/v1/auth/login/totp
#[utoipa::path(
    post,
    path = "/api/v1/auth/login/totp",
    tag = "auth_session",
    operation_id = "auth_session_login_totp",
    request_body = TotpLoginRequest,
    responses(
        (status = 200, description = "Signed in", body = LoginResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn login_totp(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(req): Json<TotpLoginRequest>,
) -> Response {
    let user = match ticket_user(&state, &req.mfa_token, MFA_TOTP).await {
        Ok(user) => user,
        Err(e) => return e.into_response(),
    };
    let subject = crate::audit::SignInSubject {
        username: user.username.clone(),
        tenant_id: Some(user.tenant_id),
    };
    let mut response = match second_factor(&state, &headers, user, &req).await {
        Ok(response) => response,
        Err(e) => e.into_response(),
    };
    response.extensions_mut().insert(subject);
    response
}

async fn second_factor(
    state: &ApiState,
    headers: &HeaderMap,
    user: fcp_core::User,
    req: &TotpLoginRequest,
) -> Result<Response, ApiError> {
    let store = totp_store(state)?;
    let totp = store
        .get_user_totp(&user.id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .filter(|t| t.is_confirmed())
        .ok_or_else(|| ApiError::Unauthorized("Sign in again".to_string()))?;
    let accepted = match (&req.code, &req.recovery_code) {
        (Some(code), _) => {
            match fcp_core::totp::matching_step(&totp.secret, code, SystemTime::now()) {
                Some(step) => store
                    .use_totp_step(&user.id, step)
                    .await
                    .map_err(|e| ApiError::Internal(e.to_string()))?
                    .then_some(fcp_core::session::method::TOTP),
                None => None,
            }
        }
        (None, Some(recovery)) => store
            .use_recovery_code(&user.id, &fcp_core::totp::recovery_hash(recovery))
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
            .then_some(fcp_core::session::method::RECOVERY_CODE),
        (None, None) => {
            return Err(ApiError::BadRequest(
                "a code or a recovery code is needed".to_string(),
            ))
        }
    };
    match accepted {
        Some(method) => complete_sign_in(state, headers, user, method, None).await,
        None => Err(count_failure(state, headers, user).await),
    }
}

/// A ticket to set a second factor up with.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct MfaTicketRequest {
    pub mfa_token: String,
}

/// A second factor being set up: the secret for the app, and the URI its
/// QR code carries.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct TotpEnrollment {
    pub secret: String,
    pub otpauth_uri: String,
}

/// A new unconfirmed secret for `user`.
pub(crate) async fn enroll(
    state: &ApiState,
    user: &fcp_core::User,
) -> Result<TotpEnrollment, ApiError> {
    let store = totp_store(state)?;
    if store
        .get_user_totp(&user.id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .is_some_and(|t| t.is_confirmed())
    {
        return Err(ApiError::Conflict(
            "A second factor is already set up; turn it off first".to_string(),
        ));
    }
    let issuer = match state.storage.as_ref() {
        Some(storage) => storage
            .get_tenant(&user.tenant_id)
            .await
            .ok()
            .flatten()
            .map(|t| format!("FCP {}", t.name))
            .unwrap_or_else(|| "FCP".to_string()),
        None => "FCP".to_string(),
    };
    let secret = fcp_core::totp::new_secret();
    store
        .put_user_totp(&fcp_core::totp::UserTotp {
            user_id: user.id,
            tenant_id: user.tenant_id,
            secret: secret.clone(),
            confirmed_at: None,
            last_step: None,
            recovery_hashes: Vec::new(),
            created_at: SystemTime::now(),
        })
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(TotpEnrollment {
        otpauth_uri: fcp_core::totp::otpauth_uri(&secret, &issuer, &user.username),
        secret,
    })
}

/// Confirm `user`'s pending secret with a code from their app: the
/// recovery codes, shown once; `None` for a wrong code.
pub(crate) async fn confirm(
    state: &ApiState,
    user: &fcp_core::User,
    code: &str,
) -> Result<Option<Vec<String>>, ApiError> {
    let store = totp_store(state)?;
    let totp = store
        .get_user_totp(&user.id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .filter(|t| !t.is_confirmed())
        .ok_or_else(|| ApiError::Conflict("Nothing is being set up".to_string()))?;
    let Some(step) = fcp_core::totp::matching_step(&totp.secret, code, SystemTime::now()) else {
        return Ok(None);
    };
    let (codes, hashes) = fcp_core::totp::new_recovery_codes();
    store
        .confirm_user_totp(&user.id, &hashes, step, SystemTime::now())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Some(codes))
}

/// POST /api/v1/auth/totp/enroll — set up the second factor a tenant
/// requires, in the middle of signing in.
#[utoipa::path(
    post,
    path = "/api/v1/auth/totp/enroll",
    tag = "auth_session",
    operation_id = "auth_session_enroll_totp",
    request_body = MfaTicketRequest,
    responses(
        (status = 200, description = "The secret to add to an app", body = TotpEnrollment),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn enroll_totp(
    State(state): State<ApiState>,
    Json(req): Json<MfaTicketRequest>,
) -> Result<Json<TotpEnrollment>, ApiError> {
    let user = ticket_user(&state, &req.mfa_token, MFA_ENROLL).await?;
    Ok(Json(enroll(&state, &user).await?))
}

/// A code from the app, proving it holds the secret.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct TotpConfirmRequest {
    pub mfa_token: String,
    pub code: String,
}

/// POST /api/v1/auth/totp/confirm — finish setting it up, and sign in.
#[utoipa::path(
    post,
    path = "/api/v1/auth/totp/confirm",
    tag = "auth_session",
    operation_id = "auth_session_confirm_totp",
    request_body = TotpConfirmRequest,
    responses(
        (status = 200, description = "Signed in, with the recovery codes", body = LoginResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn confirm_totp(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(req): Json<TotpConfirmRequest>,
) -> Result<Response, ApiError> {
    let user = ticket_user(&state, &req.mfa_token, MFA_ENROLL).await?;
    match confirm(&state, &user, &req.code).await? {
        Some(codes) => {
            complete_sign_in(
                &state,
                &headers,
                user,
                fcp_core::session::method::TOTP,
                Some(codes),
            )
            .await
        }
        None => Err(count_failure(&state, &headers, user).await),
    }
}

/// Revoke every session of `user` but `keep`: a password changed, a user
/// disabled. Logged, never fatal: the change itself has happened.
pub(crate) async fn revoke_sessions(
    state: &ApiState,
    user_id: &fcp_core::UserId,
    keep: Option<&LoginSessionId>,
    reason: &str,
) {
    let Some(store) = session_store(state) else {
        return;
    };
    match store
        .revoke_user_sessions(user_id, keep, reason, SystemTime::now())
        .await
    {
        Ok(0) => {}
        Ok(n) => tracing::info!(user_id = %user_id, sessions = n, reason, "Sessions revoked"),
        Err(e) => tracing::warn!(user_id = %user_id, error = %e, "Failed to revoke sessions"),
    }
}

pub(crate) fn publish(state: &ApiState, event: fcp_events::UserEvent) {
    let bus = std::sync::Arc::clone(&state.event_bus);
    tokio::spawn(async move {
        let _ = bus.publish(fcp_events::PlatformEvent::User(event)).await;
    });
}

/// A refused sign-in: published, and its error.
fn refused(
    state: &ApiState,
    headers: &HeaderMap,
    username: &str,
    tenant_id: Option<fcp_core::TenantId>,
    reason: &str,
    error: ApiError,
) -> ApiError {
    publish(
        state,
        fcp_events::UserEvent::SignInFailed {
            username: username.to_string(),
            tenant_id,
            reason: reason.to_string(),
            source_ip: crate::audit::source_ip(headers),
            timestamp: SystemTime::now(),
        },
    );
    error
}

/// POST /api/v1/auth/login
#[utoipa::path(
    post,
    path = "/api/v1/auth/login",
    tag = "auth_session",
    operation_id = "auth_session_login",
    request_body = LoginRequest,
    responses(
        (status = 200, description = "OK", body = LoginResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn login(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(req): Json<LoginRequest>,
) -> Response {
    let mut subject = crate::audit::SignInSubject {
        username: req.username.clone(),
        tenant_id: None,
    };
    let mut response = match sign_in(&state, &headers, req, &mut subject).await {
        Ok(response) => response,
        Err(e) => e.into_response(),
    };
    // The audit row names who tried, though the request had no identity.
    response.extensions_mut().insert(subject);
    response
}

async fn sign_in(
    state: &ApiState,
    headers: &HeaderMap,
    req: LoginRequest,
    subject: &mut crate::audit::SignInSubject,
) -> Result<Response, ApiError> {
    let storage = state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::Internal("Storage not configured".to_string()))?;

    // Get user first (we need to check lock status before password verification)
    let requested_tenant = parse_tenant_id(req.tenant_id.as_deref())?;
    let invalid = || ApiError::Unauthorized("Invalid username or password".to_string());

    let mut user =
        match load_user_for_login(storage, &req.username, requested_tenant.as_ref()).await? {
            Some(u) => u,
            None => {
                // Constant-time: still do a bcrypt check to prevent timing attacks
                let _ = password::verify_password(
                    &req.password,
                    "$2b$12$LQv3c1yqBWVHxkd0LHAkCOYz6TtxMQJqhN8/LewY5jtRKm5T6fHGC",
                );
                return Err(refused(
                    state,
                    headers,
                    &req.username,
                    None,
                    "bad_credentials",
                    invalid(),
                ));
            }
        };
    subject.tenant_id = Some(user.tenant_id);
    let tenant = Some(user.tenant_id);

    let now = SystemTime::now();
    let policy = &state.security_policy;

    // Check if account is locked
    if user.account_status == AccountStatus::Locked {
        if let Some(locked_until) = user.account_locked_until {
            if now < locked_until {
                // Still locked — do constant-time bcrypt to prevent timing attacks
                let _ = password::verify_password(&req.password, &user.password);
                return Err(refused(
                    state,
                    headers,
                    &req.username,
                    tenant,
                    "locked",
                    ApiError::Forbidden("Account is locked. Try again later.".to_string()),
                ));
            }
            // Lock expired — auto-unlock
            user.account_status = AccountStatus::Active;
            user.failed_login_attempts = 0;
            user.account_locked_until = None;
            user.updated_at = now;
            let _ = storage.put_user(user.clone()).await;
            tracing::info!(
                event = "account_auto_unlocked",
                username = %user.username,
                "Account auto-unlocked after lockout expiry"
            );
        } else {
            // Locked without expiry (admin-locked) — still do bcrypt for timing
            let _ = password::verify_password(&req.password, &user.password);
            return Err(refused(
                state,
                headers,
                &req.username,
                tenant,
                "locked",
                ApiError::Forbidden("Account is locked. Contact an administrator.".to_string()),
            ));
        }
    }

    // Check account is enabled
    if !user.enabled {
        let _ = password::verify_password(&req.password, &user.password);
        return Err(refused(
            state,
            headers,
            &req.username,
            tenant,
            "disabled",
            ApiError::Forbidden("Account is disabled".to_string()),
        ));
    }

    // Check account is not suspended
    if user.account_status == AccountStatus::Suspended {
        let _ = password::verify_password(&req.password, &user.password);
        return Err(refused(
            state,
            headers,
            &req.username,
            tenant,
            "suspended",
            ApiError::Forbidden("Account is suspended".to_string()),
        ));
    }

    // Verify password. A user without one (made by provisioning, signing
    // in through an identity provider) has nothing a password can match.
    let valid = if user.password.is_empty() {
        let _ = password::verify_password(
            &req.password,
            "$2b$12$LQv3c1yqBWVHxkd0LHAkCOYz6TtxMQJqhN8/LewY5jtRKm5T6fHGC",
        );
        false
    } else {
        password::verify_password(&req.password, &user.password)
            .map_err(|e| ApiError::Internal(e.to_string()))?
    };

    if !valid {
        // Increment failed attempts
        user.failed_login_attempts += 1;
        user.updated_at = now;

        if user.failed_login_attempts >= policy.max_failed_attempts as i32 {
            // Lock the account
            user.account_status = AccountStatus::Locked;
            user.account_locked_until =
                Some(now + Duration::from_secs(policy.lockout_duration_secs));
            tracing::warn!(
                event = "account_locked",
                username = %user.username,
                attempts = user.failed_login_attempts,
                lockout_secs = policy.lockout_duration_secs,
                "Account locked after too many failed login attempts"
            );
        }

        let _ = storage.put_user(user).await;
        // Return generic error — don't reveal lockout details to attacker
        return Err(refused(
            state,
            headers,
            &req.username,
            tenant,
            "bad_credentials",
            invalid(),
        ));
    }

    // The password is right; the tenant may still not take passwords from
    // this user (`docs/IDENTITY_AND_DEVICES.md` phase 2). Said only now, so
    // a stranger learns nothing of the policy or the account.
    let policy_login = storage
        .get_tenant(&user.tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .map(|t| t.sign_in.password_login)
        .unwrap_or_default();
    if !roles_of(state, &user)
        .await
        .iter()
        .any(|role| policy_login.permits(role))
    {
        return Err(refused(
            state,
            headers,
            &req.username,
            tenant,
            "password_not_allowed",
            ApiError::Forbidden(
                "Password sign-in is not allowed for this account; sign in with your organisation's sign-on".to_string(),
            ),
        ));
    }

    // Check password expiration
    let password_expired = if let Some(expiration_days) = policy.password_expiration_days {
        if expiration_days > 0 {
            if let Some(last_reset) = user.password_last_reset {
                let expiry = last_reset + Duration::from_secs(expiration_days as u64 * 86400);
                now > expiry
            } else {
                // No password_last_reset set — consider expired to force initial password set
                true
            }
        } else {
            false
        }
    } else {
        false
    };
    if password_expired {
        // No session until the password is changed
        // (`/auth/change-expired-password`).
        return Ok(Json(LoginResponse {
            token: None,
            expires_in: None,
            user: auth_user(&user, shown_role(state, &user).await),
            password_expired: Some(true),
            mfa_required: None,
            mfa_token: None,
            recovery_codes: None,
        })
        .into_response());
    }

    // The failed attempts are forgotten only once every factor is in: a
    // right password must not reset the count a wrong code adds to.
    after_password(state, headers, user).await
}

/// POST /api/v1/auth/logout
#[utoipa::path(
    post,
    path = "/api/v1/auth/logout",
    tag = "auth_session",
    operation_id = "auth_session_logout",
    responses(
        (status = 204, description = "Logged out: the client discards its token"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn logout(State(state): State<ApiState>, headers: HeaderMap) -> Response {
    // The session the cookie names, else the one the access token names.
    let session = match refresh_cookie(&headers) {
        Some((id, _)) => Some(id),
        None => bearer(&headers).and_then(|token| {
            let keys = state.session_keys.as_ref()?;
            keys.decode(token, SystemTime::now())
                .ok()?
                .sid?
                .parse()
                .ok()
        }),
    };
    if let (Some(id), Some(store)) = (session, session_store(&state)) {
        match store
            .revoke_login_session(&id, "signed out", SystemTime::now())
            .await
        {
            Ok(_) => tracing::info!(session = %id, "Signed out"),
            Err(e) => tracing::warn!(session = %id, error = %e, "Failed to revoke a session"),
        }
    }
    let mut response = StatusCode::NO_CONTENT.into_response();
    response
        .headers_mut()
        .append(header::SET_COOKIE, clear_cookie(&headers));
    response
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
}

/// GET /api/v1/auth/me
#[utoipa::path(
    get,
    path = "/api/v1/auth/me",
    tag = "auth_session",
    operation_id = "auth_session_me",
    responses(
        (status = 200, description = "OK", body = AuthUserResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn me(me: Me) -> Result<impl IntoResponse, ApiError> {
    if me.0.personal {
        return Err(ApiError::Unauthorized(
            "Sign in to see your session".to_string(),
        ));
    }
    Ok(Json(AuthUserResponse {
        username: me.0.sub,
        role: me.0.role,
        tenant_id: me.0.tenant_id,
        display_name: me.0.display_name,
    }))
}

/// POST /api/v1/auth/refresh
#[utoipa::path(
    post,
    path = "/api/v1/auth/refresh",
    tag = "auth_session",
    operation_id = "auth_session_refresh",
    responses(
        (status = 200, description = "A new access token; the refresh cookie turned over", body = LoginResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn refresh(State(state): State<ApiState>, headers: HeaderMap) -> Response {
    match renew(&state, &headers).await {
        Ok(response) => response,
        Err(e) => {
            // Whatever the cookie was, it opens nothing now.
            let mut response = e.into_response();
            response
                .headers_mut()
                .append(header::SET_COOKIE, clear_cookie(&headers));
            response
        }
    }
}

async fn renew(state: &ApiState, headers: &HeaderMap) -> Result<Response, ApiError> {
    let unauthorized = |why: &str| ApiError::Unauthorized(why.to_string());
    let (id, secret) = refresh_cookie(headers).ok_or_else(|| unauthorized("Not signed in"))?;
    let store = session_store(state).ok_or_else(|| unauthorized("Not signed in"))?;
    let storage = state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::Internal("Storage not configured".to_string()))?;
    let now = SystemTime::now();
    let (next_secret, next_hash) = session::new_refresh_secret();
    let outcome = store
        .rotate_refresh(&id, &session::refresh_hash(&secret), &next_hash, now)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    match outcome {
        RefreshOutcome::Rotated | RefreshOutcome::RecentlyRotated => {}
        RefreshOutcome::Reused => {
            tracing::warn!(session = %id, "A refresh secret was used twice: the session is revoked");
            return Err(unauthorized("Session ended"));
        }
        RefreshOutcome::Unknown => return Err(unauthorized("Session ended")),
    }
    let session = store
        .get_login_session(&id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .filter(|s| s.is_live(now))
        .ok_or_else(|| unauthorized("Session ended"))?;
    let user = storage
        .get_user_by_id(&session.user_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .filter(|u| u.tenant_id == session.tenant_id && session::user_is_active(u))
        .ok_or_else(|| unauthorized("Session ended"))?;
    let (token, expires_in) = access_token(state, &user, Some(&id), now)?;
    let issued = Issued {
        token,
        expires_in,
        // A secret turned over goes back in the cookie; a second tab that
        // presented the one before it keeps the cookie the first tab set.
        cookie: (outcome == RefreshOutcome::Rotated).then(|| {
            let left = session
                .expires_at
                .duration_since(now)
                .unwrap_or_default()
                .as_secs();
            cookie(
                &session::refresh_cookie_value(&id, &next_secret),
                left,
                is_https(headers),
            )
        }),
        session: Some(id),
    };
    let role = shown_role(state, &user).await;
    Ok(signed_in(issued, &user, role, false))
}

/// POST /api/v1/auth/change-expired-password
///
/// Allows a user with an expired password to set a new password.
/// Requires current credentials for verification.
#[utoipa::path(
    post,
    path = "/api/v1/auth/change-expired-password",
    tag = "auth_session",
    operation_id = "auth_session_change_expired_password",
    request_body = ChangeExpiredPasswordRequest,
    responses(
        (status = 200, description = "OK", body = LoginResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn change_expired_password(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(req): Json<ChangeExpiredPasswordRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::Internal("Storage not configured".to_string()))?;

    // Validate new password is not empty
    if req.new_password.is_empty() {
        return Err(ApiError::BadRequest(
            "New password cannot be empty".to_string(),
        ));
    }

    // Validate new password is different from current
    if req.new_password == req.current_password {
        return Err(ApiError::BadRequest(
            "New password must be different from current password".to_string(),
        ));
    }

    // Get user (with constant-time bcrypt on miss to prevent timing attacks)
    let requested_tenant = parse_tenant_id(req.tenant_id.as_deref())?;

    let mut user =
        match load_user_for_login(storage, &req.username, requested_tenant.as_ref()).await? {
            Some(u) => u,
            None => {
                let _ = password::verify_password(
                    &req.current_password,
                    "$2b$12$LQv3c1yqBWVHxkd0LHAkCOYz6TtxMQJqhN8/LewY5jtRKm5T6fHGC",
                );
                return Err(ApiError::Unauthorized(
                    "Invalid username or password".to_string(),
                ));
            }
        };

    let now = SystemTime::now();
    let policy = &state.security_policy;

    // Check if account is locked
    if user.account_status == AccountStatus::Locked {
        if let Some(locked_until) = user.account_locked_until {
            if now < locked_until {
                let _ = password::verify_password(&req.current_password, &user.password);
                return Err(ApiError::Forbidden(
                    "Account is locked. Try again later.".to_string(),
                ));
            }
            // Lock expired — auto-unlock
            user.account_status = AccountStatus::Active;
            user.failed_login_attempts = 0;
            user.account_locked_until = None;
            user.updated_at = now;
            let _ = storage.put_user(user.clone()).await;
            tracing::info!(
                event = "account_auto_unlocked",
                username = %user.username,
                "Account auto-unlocked after lockout expiry"
            );
        } else {
            let _ = password::verify_password(&req.current_password, &user.password);
            return Err(ApiError::Forbidden(
                "Account is locked. Contact an administrator.".to_string(),
            ));
        }
    }

    // Check account is enabled
    if !user.enabled {
        let _ = password::verify_password(&req.current_password, &user.password);
        return Err(ApiError::Forbidden("Account is disabled".to_string()));
    }

    // Check account is not suspended
    if user.account_status == AccountStatus::Suspended {
        let _ = password::verify_password(&req.current_password, &user.password);
        return Err(ApiError::Forbidden("Account is suspended".to_string()));
    }

    // Verify current password
    let valid = password::verify_password(&req.current_password, &user.password)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    if !valid {
        // Increment failed attempts
        user.failed_login_attempts += 1;
        user.updated_at = now;

        if user.failed_login_attempts >= policy.max_failed_attempts as i32 {
            user.account_status = AccountStatus::Locked;
            user.account_locked_until =
                Some(now + Duration::from_secs(policy.lockout_duration_secs));
            tracing::warn!(
                event = "account_locked",
                username = %user.username,
                attempts = user.failed_login_attempts,
                lockout_secs = policy.lockout_duration_secs,
                "Account locked after too many failed password change attempts"
            );
        }

        let _ = storage.put_user(user).await;
        return Err(ApiError::Unauthorized(
            "Invalid username or password".to_string(),
        ));
    }

    // Password correct — reset failed attempts
    user.failed_login_attempts = 0;

    // Verify the password is actually expired (prevent using this as a
    // general password-change path that bypasses the /me/password flow)
    if let Some(expiration_days) = state.security_policy.password_expiration_days {
        if expiration_days > 0 {
            let expired = if let Some(last_reset) = user.password_last_reset {
                let expiry = last_reset + Duration::from_secs(expiration_days as u64 * 86400);
                now > expiry
            } else {
                true // No reset date → treat as expired
            };
            if !expired {
                return Err(ApiError::BadRequest(
                    "Password is not expired. Use the regular password change endpoint."
                        .to_string(),
                ));
            }
        } else {
            return Err(ApiError::BadRequest(
                "Password expiration is not enabled".to_string(),
            ));
        }
    } else {
        return Err(ApiError::BadRequest(
            "Password expiration is not enabled".to_string(),
        ));
    }

    // Hash new password
    let password_hash = password::hash_password(&req.new_password)
        .map_err(|e| ApiError::Internal(format!("Failed to hash password: {}", e)))?;

    // Update user
    user.password = password_hash;
    if user.digest_password.is_none() {
        user.sip_ha1_hash = Some(password::compute_ha1(
            user.sip_name(),
            &state.realm,
            &req.new_password,
        ));
        user.sip_provisioning_password = None;
    }
    user.password_last_reset = Some(now);
    user.last_login = Some(now);
    user.updated_at = now;

    storage
        .put_user(user.clone())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    tracing::warn!(
        event = "expired_password_changed",
        username = %user.username,
        "User changed expired password"
    );

    // Whoever else held the old password is signed out; then the second
    // factor, or a session.
    revoke_sessions(&state, &user.id, None, "password changed").await;
    after_password(&state, &headers, user).await
}

// ---- Who is calling ----

/// The caller of a `/me` route, as the claims its handler reads: a
/// signed-in user as the bearer layer found them, or — for a personal API
/// token (docs/DEVELOPER_SURFACE.md §3, phase 1) — its maker, as an end
/// user. With authentication off there is no bearer layer, and the access
/// token is checked here the same way.
pub struct Me(pub Claims);

#[axum::async_trait]
impl axum::extract::FromRequestParts<ApiState> for Me {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &ApiState,
    ) -> Result<Self, Self::Rejection> {
        use crate::auth::AuthIdentity;
        match parts.extensions.get::<AuthIdentity>() {
            Some(AuthIdentity::Personal {
                username,
                tenant_id,
                user_id,
                ..
            }) => {
                return Ok(Me(Claims {
                    sub: username.clone(),
                    role: "end_user".to_string(),
                    tenant_id: tenant_id.to_string(),
                    display_name: None,
                    exp: 0,
                    iat: 0,
                    user_id: user_id.map(|id| id.to_string()),
                    sid: None,
                    personal: true,
                }))
            }
            Some(
                identity @ AuthIdentity::Session {
                    username,
                    tenant_id: Some(tenant_id),
                    user_id,
                    session_id,
                    ..
                },
            ) => {
                return Ok(Me(Claims {
                    sub: username.clone(),
                    // The most privileged of their own role and their
                    // groups', to show; decisions ask each.
                    role: display_role(&identity.session_roles().unwrap_or_default()),
                    tenant_id: tenant_id.to_string(),
                    display_name: None,
                    exp: 0,
                    iat: 0,
                    user_id: user_id.map(|id| id.to_string()),
                    sid: session_id.map(|id| id.to_string()),
                    personal: false,
                }));
            }
            _ => {}
        }
        let token = bearer(&parts.headers)
            .ok_or_else(|| ApiError::Unauthorized("Missing Authorization header".to_string()))?;
        let users = state
            .storage
            .as_ref()
            .map(|s| s.credential_store_arc())
            .ok_or_else(|| ApiError::Internal("Storage not configured".to_string()))?;
        let sessions = session_store(state);
        let principal = session::authenticate_session(
            token,
            keys(state)?,
            users.as_ref(),
            sessions.as_deref(),
            SystemTime::now(),
        )
        .await
        .ok_or_else(|| ApiError::Unauthorized("Invalid or expired session".to_string()))?;
        let user = principal.user;
        let mut roles = vec![user.role];
        roles.extend(principal.group_roles);
        Ok(Me(Claims {
            sub: user.username,
            role: display_role(&roles),
            tenant_id: user.tenant_id.to_string(),
            display_name: user.display_name,
            exp: 0,
            iat: 0,
            user_id: Some(user.id.to_string()),
            sid: principal.session.map(|id| id.to_string()),
            personal: false,
        }))
    }
}
