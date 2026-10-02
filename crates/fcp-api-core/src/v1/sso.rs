//! Single sign-on through a tenant's identity providers
//! (`docs/IDENTITY_AND_DEVICES.md` phase 3).
//!
//! The browser goes to `/auth/sso/{provider}/start`, which remembers the
//! sign-in (its state, nonce and PKCE verifier), sets a cookie naming it
//! and sends the browser to the provider. The provider sends it back to
//! `/auth/sso/callback`, which wants that cookie — a callback someone else
//! started is refused — trades the code for an ID token, verifies it, and
//! finds the user: by their identity at the provider, else by the verified
//! email the provider asserts, else made just in time if the provider says
//! so. The browser lands on the login page with a one-time code, which the
//! UI exchanges at `/auth/sso/exchange` for a session like any other.
//!
//! Providers are a tenant's to manage (`identity_provider_manage`): the
//! client secret is written, never read back.

use crate::{
    auth::AuthIdentity,
    error::ApiError,
    oidc::{Begun, OidcError},
    state::{ApiState, SsoRuntime},
    v1::auth_session::{complete_sign_in_through, publish},
};
use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Redirect, Response},
    Extension, Json,
};
use fcp_core::identity::{
    IdentityProvider, IdentityProviderId, IdentityStore, ProviderKind, ProviderSettings, SsoLogin,
    UserIdentity,
};
use fcp_core::permissions::Permission;
use fcp_core::{session, CredentialStore, TenantStore, User, UserRole};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::SystemTime;

/// The cookie that ties a callback to the browser that began the sign-in.
const STATE_COOKIE: &str = "fcp_sso";
const STATE_COOKIE_PATH: &str = "/api/v1/auth/sso";

fn identity_store(state: &ApiState) -> Result<Arc<dyn IdentityStore>, ApiError> {
    state
        .storage
        .as_ref()
        .and_then(|s| s.identity_store().cloned())
        .ok_or_else(|| ApiError::Internal("Identity providers are not available".to_string()))
}

fn storage(state: &ApiState) -> Result<&Arc<fcp_core::DynStorage>, ApiError> {
    state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::Internal("Storage not configured".to_string()))
}

fn sso(state: &ApiState) -> Result<&Arc<SsoRuntime>, ApiError> {
    state.sso.as_ref().ok_or_else(|| {
        ApiError::ServiceUnavailable(
            "Single sign-on is not configured here ([api.sso] public_url)".to_string(),
        )
    })
}

fn hash(code: &str) -> String {
    Sha256::digest(code.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A tenant's providers a person may sign in with.
pub(crate) async fn offered(
    state: &ApiState,
    tenant_id: &fcp_core::TenantId,
) -> Vec<IdentityProvider> {
    if state.sso.is_none() {
        return Vec::new();
    }
    let Ok(store) = identity_store(state) else {
        return Vec::new();
    };
    store
        .list_identity_providers(tenant_id)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|p| p.enabled)
        .collect()
}

// ---- Signing in ----

/// GET /api/v1/auth/sso/{id}/start — off to the provider.
#[utoipa::path(
    get,
    path = "/api/v1/auth/sso/{id}/start",
    tag = "auth_session",
    operation_id = "auth_session_sso_start",
    params(("id" = String, Path, description = "The identity provider")),
    responses(
        (status = 302, description = "To the provider's sign-in page"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn start(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let sso = sso(&state)?.clone();
    let store = identity_store(&state)?;
    let not_found = || ApiError::NotFound("No such identity provider".to_string());
    let id: IdentityProviderId = id.parse().map_err(|_| not_found())?;
    let provider = store
        .get_identity_provider(&id)
        .await?
        .filter(|p| p.enabled)
        .ok_or_else(not_found)?;
    let discovery = sso
        .client
        .discovery(&provider.settings.issuer)
        .await
        .map_err(|e| ApiError::ServiceUnavailable(e.to_string()))?;
    let begun = Begun::new();
    let url = sso
        .client
        .authorize_url(&discovery, &provider.settings, &sso.redirect_uri(), &begun)
        .map_err(|e| ApiError::ServiceUnavailable(e.to_string()))?;
    store
        .create_sso_login(&SsoLogin {
            state: begun.state.clone(),
            provider_id: provider.id,
            tenant_id: provider.tenant_id,
            nonce: begun.nonce,
            code_verifier: begun.code_verifier,
            created_at: SystemTime::now(),
        })
        .await?;
    // Lax, not Strict: the provider's redirect back is a top-level
    // navigation from another site, which must carry it.
    let secure = if sso.public_url.starts_with("https://") {
        "; Secure"
    } else {
        ""
    };
    let cookie = HeaderValue::from_str(&format!(
        "{STATE_COOKIE}={}; Path={STATE_COOKIE_PATH}; Max-Age={}; HttpOnly; SameSite=Lax{secure}",
        begun.state,
        fcp_core::identity::SSO_LOGIN_TTL.as_secs()
    ))
    .expect("base64url is a header value");
    let mut response = Redirect::to(&url).into_response();
    response.headers_mut().insert(header::SET_COOKIE, cookie);
    Ok(response)
}

/// What the provider sends back.
#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct CallbackQuery {
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
    /// The provider's refusal (`access_denied`, …).
    #[serde(default)]
    pub error: Option<String>,
}

fn state_cookie(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .find_map(|pair| {
            let (name, value) = pair.trim().split_once('=')?;
            (name == STATE_COOKIE).then(|| value.to_string())
        })
}

/// GET /api/v1/auth/sso/callback — back from the provider: to the login
/// page with a one-time code, or with the reason it failed.
#[utoipa::path(
    get,
    path = "/api/v1/auth/sso/callback",
    tag = "auth_session",
    operation_id = "auth_session_sso_callback",
    params(CallbackQuery),
    responses(
        (status = 302, description = "To the login page, with `sso_code` or `sso_error`"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn callback(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<CallbackQuery>,
) -> Result<Response, ApiError> {
    let sso = sso(&state)?.clone();
    let landing = |param: &str, value: &str| {
        let mut response = Redirect::to(&format!(
            "{}/login?{param}={}",
            sso.public_url,
            urlencoding::encode(value)
        ))
        .into_response();
        // The sign-in is over either way.
        response.headers_mut().insert(
            header::SET_COOKIE,
            HeaderValue::from_static(
                "fcp_sso=; Path=/api/v1/auth/sso; Max-Age=0; HttpOnly; SameSite=Lax",
            ),
        );
        response
    };

    // The state must be the one this browser began with, and still waiting.
    let Some(sent) = q.state.as_deref().filter(|s| !s.is_empty()) else {
        return Ok(landing("sso_error", "bad_state"));
    };
    if state_cookie(&headers).as_deref() != Some(sent) {
        return Ok(landing("sso_error", "bad_state"));
    }
    let store = identity_store(&state)?;
    let Some(login) = store.take_sso_login(sent, SystemTime::now()).await? else {
        return Ok(landing("sso_error", "expired"));
    };
    let Some(provider) = store.get_identity_provider(&login.provider_id).await? else {
        return Ok(landing("sso_error", "expired"));
    };
    let fail = |reason: &str, username: Option<&str>| {
        publish(
            &state,
            fcp_events::UserEvent::SignInFailed {
                username: username.unwrap_or_default().to_string(),
                tenant_id: Some(provider.tenant_id),
                reason: reason.to_string(),
                source_ip: crate::audit::source_ip(&headers),
                timestamp: SystemTime::now(),
            },
        );
        tracing::warn!(provider = %provider.id, tenant_id = %provider.tenant_id, reason, "Single sign-on refused");
        landing("sso_error", reason)
    };
    if let Some(error) = q.error.as_deref() {
        tracing::info!(provider = %provider.id, error, "The identity provider refused the sign-in");
        return Ok(fail("provider_refused", None));
    }
    let (Some(code), true) = (q.code.as_deref(), provider.enabled) else {
        return Ok(fail("provider_refused", None));
    };

    let claims = match sso
        .client
        .sign_in(
            &provider.settings,
            provider.client_secret.as_deref(),
            code,
            &login.code_verifier,
            &sso.redirect_uri(),
            &login.nonce,
        )
        .await
    {
        Ok(claims) => claims,
        Err(e) => {
            tracing::warn!(provider = %provider.id, error = %e, "Single sign-on failed");
            return Ok(fail(e.reason(), None));
        }
    };

    let user = match resolve_user(&state, &provider, &claims).await {
        Ok(user) => user,
        Err(Refusal(reason, username)) => return Ok(fail(reason, username.as_deref())),
    };
    let handoff = crate::oidc::random_token();
    store
        .put_sso_handoff(
            &fcp_core::identity::SsoHandoff {
                code_hash: hash(&handoff),
                user_id: user.id,
                provider_id: provider.id,
                idp_sid: claim_str(&claims, "sid"),
            },
            SystemTime::now(),
        )
        .await?;
    Ok(landing("sso_code", &handoff))
}

/// Why a verified sign-in found no user to be, and whom it named.
struct Refusal(&'static str, Option<String>);

fn claim_str(claims: &Map<String, Value>, name: &str) -> Option<String> {
    claims
        .get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// The groups claim: a list of strings, or one.
fn claim_groups(claims: &Map<String, Value>, name: &str) -> Option<Vec<String>> {
    match claims.get(name)? {
        Value::Array(list) => Some(
            list.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect(),
        ),
        Value::String(one) => Some(vec![one.clone()]),
        _ => None,
    }
}

fn email_verified(claims: &Map<String, Value>, claim: &str) -> bool {
    match claims.get(claim) {
        Some(Value::Bool(b)) => *b,
        // Some providers send it as a string.
        Some(Value::String(s)) => s == "true",
        _ => false,
    }
}

/// The username a person made just in time gets: the claim, or its part
/// before the `@` when it is an address.
fn jit_username(candidate: &str) -> Option<String> {
    let name = candidate.split('@').next().unwrap_or_default().trim();
    let name: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "._-".contains(c) {
                c
            } else {
                '.'
            }
        })
        .collect();
    let ok = crate::validation::validate_username(&name).is_ok()
        && !(name.starts_with("user-") && name.parse::<fcp_core::UserId>().is_ok());
    ok.then_some(name)
}

/// The user a verified sign-in is: by identity, by verified email, or new.
async fn resolve_user(
    state: &ApiState,
    provider: &IdentityProvider,
    claims: &Map<String, Value>,
) -> Result<User, Refusal> {
    let internal = |_| Refusal("internal", None);
    let storage = storage(state).map_err(internal)?;
    let store = identity_store(state).map_err(internal)?;
    let mapping = &provider.settings.claims;
    let subject = claim_str(claims, "sub").ok_or(Refusal("bad_id_token", None))?;
    // A site's users come from the cloud (docs/HYBRID_CONTROL_PLANE.md):
    // it links them, but neither makes them nor sets their roles.
    let at_site = state.managed_by.is_some();
    let email = claim_str(claims, &mapping.email);
    let username_claim = claim_str(claims, &mapping.username);
    let display_name = claim_str(claims, &mapping.display_name);
    let named = username_claim.clone().or_else(|| email.clone());
    let refuse = |reason: &'static str| Refusal(reason, named.clone());
    // Only the people the tenant meant: its Workspace domain, its
    // directory.
    if !provider.settings.carries_required(claims) {
        return Err(refuse("claim_mismatch"));
    }

    let known = store
        .get_user_identity(&provider.id, &subject)
        .await
        .map_err(|_| refuse("internal"))?;
    let mut linked = known.is_some();
    let mut user = match &known {
        Some(identity) => storage
            .get_user_by_id(&identity.user_id)
            .await
            .map_err(|_| refuse("internal"))?
            .filter(|u| u.tenant_id == provider.tenant_id),
        None => None,
    };
    let now = SystemTime::now();

    // Someone the provider vouches for, new here: their verified email
    // links the user that has it, if exactly one does.
    if user.is_none()
        && provider.settings.link_by_email
        && email_verified(claims, &provider.settings.email_verified_claim)
    {
        if let Some(email) = &email {
            let found = storage
                .find_users_by_email_in_tenant(email, &provider.tenant_id)
                .await
                .map_err(|_| refuse("internal"))?;
            if found.len() == 1 {
                user = found.into_iter().next();
                linked = false;
            }
        }
    }
    let groups = claim_groups(claims, &mapping.groups);
    let mapped_role = groups
        .as_deref()
        .map(|g| provider.settings.role_for(g).unwrap_or(UserRole::EndUser));

    let mut user = match user {
        Some(user) => user,
        None if provider.settings.jit && !at_site => {
            let username = username_claim
                .as_deref()
                .and_then(jit_username)
                .or_else(|| email.as_deref().and_then(jit_username))
                .ok_or_else(|| refuse("no_username"))?;
            if storage
                .get_user_in_tenant(&username, &provider.tenant_id)
                .await
                .map_err(|_| refuse("internal"))?
                .is_some()
            {
                return Err(refuse("username_taken"));
            }
            let role = if provider.settings.group_roles.is_empty() {
                UserRole::EndUser
            } else {
                mapped_role.unwrap_or(UserRole::EndUser)
            };
            let user = crate::v1::users::user_without_password(
                &state.realm,
                provider.tenant_id,
                &username,
                display_name.clone(),
                email.clone(),
                role,
            )
            .map_err(|_| refuse("internal"))?;
            storage
                .put_user(user.clone())
                .await
                .map_err(|_| refuse("internal"))?;
            publish(
                state,
                fcp_events::UserEvent::Created {
                    user_id: user.id.to_string(),
                    username: user.username.clone(),
                    tenant_id: user.tenant_id,
                    timestamp: now,
                },
            );
            tracing::info!(username = %user.username, tenant_id = %user.tenant_id, provider = %provider.id, "User created at first sign-in");
            user
        }
        None => return Err(refuse("not_provisioned")),
    };
    let refuse = |reason: &'static str| Refusal(reason, Some(user.username.clone()));
    if !session::user_is_active(&user) {
        return Err(refuse("disabled"));
    }
    // A tenant's provider is the tenant admin's to configure: were it to
    // sign in a platform administrator (by asserting their email), a
    // tenant admin would become one. Super admins sign in with a password.
    if user.role == UserRole::SuperAdmin {
        return Err(refuse("super_admin"));
    }

    // The provider's groups decide the role, where it maps any — at the
    // cloud, the users' system of record.
    if let (Some(role), false, false) = (
        mapped_role,
        provider.settings.group_roles.is_empty(),
        at_site,
    ) {
        if user.role != role {
            tracing::info!(username = %user.username, from = ?user.role, to = ?role, "Role set by the identity provider's groups");
            user.role = role;
            user.updated_at = now;
            storage
                .put_user(user.clone())
                .await
                .map_err(|_| refuse("internal"))?;
        }
    }

    store
        .put_user_identity(&UserIdentity {
            provider_id: provider.id,
            subject,
            user_id: user.id,
            tenant_id: user.tenant_id,
            email,
            created_at: known.as_ref().map_or(now, |k| k.created_at),
            last_sign_in_at: Some(now),
        })
        .await
        .map_err(|_| refuse("internal"))?;
    if !linked {
        tracing::info!(username = %user.username, provider = %provider.id, "Identity linked");
    }
    Ok(user)
}

/// The one-time code the login page was handed.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct SsoExchangeRequest {
    pub code: String,
}

/// POST /api/v1/auth/sso/exchange — a session for the one-time code.
#[utoipa::path(
    post,
    path = "/api/v1/auth/sso/exchange",
    tag = "auth_session",
    operation_id = "auth_session_sso_exchange",
    request_body = SsoExchangeRequest,
    responses(
        (status = 200, description = "Signed in", body = crate::v1::auth_session::LoginResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn exchange(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(req): Json<SsoExchangeRequest>,
) -> Result<Response, ApiError> {
    let expired = || ApiError::Unauthorized("Sign in again".to_string());
    let handoff = identity_store(&state)?
        .take_sso_handoff(&hash(&req.code), SystemTime::now())
        .await?
        .ok_or_else(expired)?;
    let user = storage(&state)?
        .get_user_by_id(&handoff.user_id)
        .await?
        .filter(session::user_is_active)
        .ok_or_else(expired)?;
    let subject = crate::audit::SignInSubject {
        username: user.username.clone(),
        tenant_id: Some(user.tenant_id),
    };
    let idp = session::IdpSession {
        provider_id: handoff.provider_id.to_string(),
        sid: handoff.idp_sid,
    };
    let mut response = complete_sign_in_through(
        &state,
        &headers,
        user,
        session::method::OIDC,
        None,
        Some(idp),
    )
    .await?;
    response.extensions_mut().insert(subject);
    Ok(response)
}

/// A provider's back-channel logout request.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct BackchannelLogoutForm {
    pub logout_token: String,
}

/// POST /api/v1/auth/sso/{id}/backchannel-logout — the provider says a
/// person signed out there (OpenID Connect Back-Channel Logout 1.0): the
/// sessions begun through it as that provider session (`sid`), or all of
/// that person's (`sub`), end.
#[utoipa::path(
    post,
    path = "/api/v1/auth/sso/{id}/backchannel-logout",
    tag = "auth_session",
    operation_id = "auth_session_sso_backchannel_logout",
    params(("id" = String, Path, description = "The identity provider")),
    request_body(content = BackchannelLogoutForm, content_type = "application/x-www-form-urlencoded"),
    responses(
        (status = 200, description = "The sessions it names are ended"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn backchannel_logout(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    axum::Form(form): axum::Form<BackchannelLogoutForm>,
) -> Result<Response, ApiError> {
    let sso = sso(&state)?.clone();
    let store = identity_store(&state)?;
    let not_found = || ApiError::NotFound("No such identity provider".to_string());
    let id: IdentityProviderId = id.parse().map_err(|_| not_found())?;
    let provider = store
        .get_identity_provider(&id)
        .await?
        .ok_or_else(not_found)?;
    let discovery = sso
        .client
        .discovery(&provider.settings.issuer)
        .await
        .map_err(|e| ApiError::ServiceUnavailable(e.to_string()))?;
    let claims = sso
        .client
        .verify_logout(
            &discovery,
            &provider.settings.client_id,
            &form.logout_token,
            SystemTime::now(),
        )
        .await
        .map_err(|e| {
            tracing::warn!(provider = %provider.id, error = %e, "A back-channel logout refused");
            ApiError::BadRequest(e.to_string())
        })?;
    let sid = claim_str(&claims, "sid");
    let user = match claim_str(&claims, "sub") {
        Some(sub) => store
            .get_user_identity(&provider.id, &sub)
            .await?
            .map(|i| i.user_id),
        None => None,
    };
    // A `sub` nobody here is: nothing to end, unless the `sid` names a
    // session on its own.
    if user.is_none() && sid.is_none() {
        return Ok(no_store(StatusCode::OK));
    }
    let ended = match state
        .storage
        .as_ref()
        .and_then(|s| s.login_session_store().cloned())
    {
        Some(sessions) => {
            sessions
                .revoke_idp_sessions(
                    &provider.id.to_string(),
                    user.as_ref(),
                    sid.as_deref(),
                    "signed out at the identity provider",
                    SystemTime::now(),
                )
                .await?
        }
        None => 0,
    };
    tracing::info!(provider = %provider.id, ended, "Back-channel logout");
    Ok(no_store(StatusCode::OK))
}

/// A response the provider must not cache (Back-Channel Logout §2.8).
fn no_store(status: StatusCode) -> Response {
    let mut response = status.into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

// ---- The tenant's providers ----

/// An identity provider, its client secret withheld.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct IdentityProviderResponse {
    pub id: String,
    pub tenant_id: String,
    pub name: String,
    #[schema(value_type = String)]
    pub kind: ProviderKind,
    pub enabled: bool,
    pub settings: ProviderSettings,
    /// Whether a client secret is set (it is never shown).
    pub has_client_secret: bool,
    /// The address to register with the provider as the redirect URI;
    /// absent where `[api.sso] public_url` is not set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redirect_uri: Option<String>,
    /// The address to register as its back-channel logout URI.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backchannel_logout_uri: Option<String>,
    #[serde(with = "crate::time::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub created_at: SystemTime,
    #[serde(with = "crate::time::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: SystemTime,
}

impl IdentityProviderResponse {
    fn from(state: &ApiState, p: IdentityProvider) -> Self {
        Self {
            id: p.id.to_string(),
            tenant_id: p.tenant_id.to_string(),
            name: p.name,
            kind: p.kind,
            enabled: p.enabled,
            settings: p.settings,
            has_client_secret: p.client_secret.is_some(),
            redirect_uri: state.sso.as_ref().map(|s| s.redirect_uri()),
            backchannel_logout_uri: state.sso.as_ref().map(|s| {
                format!(
                    "{}/api/v1/auth/sso/{}/backchannel-logout",
                    s.public_url, p.id
                )
            }),
            created_at: p.created_at,
            updated_at: p.updated_at,
        }
    }
}

/// A provider written: its settings, and the client secret — absent to
/// keep the one it has, empty to remove it.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct IdentityProviderRequest {
    pub name: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    pub settings: ProviderSettings,
    #[serde(default)]
    pub client_secret: Option<String>,
}

fn yes() -> bool {
    true
}

fn check(state: &ApiState, req: &IdentityProviderRequest) -> Result<(), ApiError> {
    if req.name.trim().is_empty() || req.name.len() > 100 {
        return Err(ApiError::BadRequest(
            "name is required (100 characters at most)".into(),
        ));
    }
    if let Some(problem) = req.settings.problems() {
        return Err(ApiError::BadRequest(problem));
    }
    let allow_private = state
        .sso
        .as_ref()
        .map(|s| s.client.check_issuer(&req.settings.issuer));
    match allow_private {
        Some(Err(e)) => Err(ApiError::BadRequest(format!("issuer: {e}"))),
        Some(Ok(())) => Ok(()),
        // No SSO here: judge it as a public issuer would be.
        None => crate::oidc::OidcClient::new(false)
            .check_issuer(&req.settings.issuer)
            .map_err(|e| ApiError::BadRequest(format!("issuer: {e}"))),
    }
}

async fn tenant_provider(
    state: &ApiState,
    tenant: &fcp_core::TenantId,
    id: &str,
) -> Result<IdentityProvider, ApiError> {
    let not_found = || ApiError::NotFound(format!("Identity provider '{id}' not found"));
    let id: IdentityProviderId = id.parse().map_err(|_| not_found())?;
    identity_store(state)?
        .get_identity_provider(&id)
        .await?
        .filter(|p| &p.tenant_id == tenant)
        .ok_or_else(not_found)
}

/// Refuse taking away the last provider of a tenant that keeps passwords
/// from some of its people: they would have no way in.
async fn ensure_a_way_in(
    state: &ApiState,
    tenant: &fcp_core::TenantId,
    leaving: &IdentityProviderId,
) -> Result<(), ApiError> {
    let restricted = storage(state)?
        .get_tenant(tenant)
        .await?
        .is_some_and(|t| t.sign_in.password_login != fcp_core::PasswordLogin::Allowed);
    if !restricted {
        return Ok(());
    }
    let others = identity_store(state)?
        .list_identity_providers(tenant)
        .await?
        .into_iter()
        .any(|p| p.enabled && &p.id != leaving);
    if others {
        Ok(())
    } else {
        Err(ApiError::Conflict(
            "This is the tenant's last identity provider and its password policy keeps passwords from some users: allow passwords first".into(),
        ))
    }
}

/// GET /api/v1/identity-providers
#[utoipa::path(
    get,
    path = "/api/v1/identity-providers",
    tag = "identity_providers",
    operation_id = "identity_providers_list",
    params(crate::page::PageQuery),
    responses(
        (status = 200, description = "OK", body = crate::page::Page<IdentityProviderResponse>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Query(page): Query<crate::page::PageQuery>,
) -> Result<Json<crate::page::Page<IdentityProviderResponse>>, ApiError> {
    crate::auth::check_permission(
        &identity,
        Permission::IdentityProviderManage,
        state.deployment_mode,
    )?;
    let providers = identity_store(&state)?
        .list_identity_providers(&tenant.0)
        .await?;
    let items: Vec<IdentityProviderResponse> = providers
        .into_iter()
        .map(|p| IdentityProviderResponse::from(&state, p))
        .collect();
    Ok(Json(crate::page::paginate(
        items,
        &page,
        crate::page::Order::Ascending,
        |p| format!("{}\u{0}{}", p.name, p.id),
    )?))
}

/// GET /api/v1/identity-providers/{id}
#[utoipa::path(
    get,
    path = "/api/v1/identity-providers/{id}",
    tag = "identity_providers",
    operation_id = "identity_providers_get",
    params(("id" = String, Path)),
    responses(
        (status = 200, description = "OK", body = IdentityProviderResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn get(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
) -> Result<Json<IdentityProviderResponse>, ApiError> {
    crate::auth::check_permission(
        &identity,
        Permission::IdentityProviderManage,
        state.deployment_mode,
    )?;
    let provider = tenant_provider(&state, &tenant.0, &id).await?;
    Ok(Json(IdentityProviderResponse::from(&state, provider)))
}

/// POST /api/v1/identity-providers
#[utoipa::path(
    post,
    path = "/api/v1/identity-providers",
    tag = "identity_providers",
    operation_id = "identity_providers_create",
    request_body = IdentityProviderRequest,
    responses(
        (status = 201, description = "Created", body = IdentityProviderResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn create(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Json(req): Json<IdentityProviderRequest>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(
        &identity,
        Permission::IdentityProviderManage,
        state.deployment_mode,
    )?;
    check(&state, &req)?;
    let now = SystemTime::now();
    let provider = IdentityProvider {
        id: IdentityProviderId::new(),
        tenant_id: tenant.0,
        name: req.name.trim().to_string(),
        kind: ProviderKind::Oidc,
        enabled: req.enabled,
        settings: req.settings,
        client_secret: req.client_secret.filter(|s| !s.is_empty()),
        created_at: now,
        updated_at: now,
    };
    identity_store(&state)?
        .put_identity_provider(&provider)
        .await?;
    tracing::info!(provider = %provider.id, tenant_id = %provider.tenant_id, by = %identity.actor(), "Identity provider created");
    Ok((
        StatusCode::CREATED,
        Json(IdentityProviderResponse::from(&state, provider)),
    ))
}

/// PUT /api/v1/identity-providers/{id}
#[utoipa::path(
    put,
    path = "/api/v1/identity-providers/{id}",
    tag = "identity_providers",
    operation_id = "identity_providers_update",
    params(("id" = String, Path)),
    request_body = IdentityProviderRequest,
    responses(
        (status = 200, description = "OK", body = IdentityProviderResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn update(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
    Json(req): Json<IdentityProviderRequest>,
) -> Result<Json<IdentityProviderResponse>, ApiError> {
    crate::auth::check_permission(
        &identity,
        Permission::IdentityProviderManage,
        state.deployment_mode,
    )?;
    check(&state, &req)?;
    let mut provider = tenant_provider(&state, &tenant.0, &id).await?;
    if provider.enabled && !req.enabled {
        ensure_a_way_in(&state, &tenant.0, &provider.id).await?;
    }
    provider.name = req.name.trim().to_string();
    provider.enabled = req.enabled;
    provider.settings = req.settings;
    match req.client_secret {
        Some(s) if s.is_empty() => provider.client_secret = None,
        Some(s) => provider.client_secret = Some(s),
        None => {}
    }
    provider.updated_at = SystemTime::now();
    identity_store(&state)?
        .put_identity_provider(&provider)
        .await?;
    tracing::info!(provider = %provider.id, by = %identity.actor(), "Identity provider updated");
    Ok(Json(IdentityProviderResponse::from(&state, provider)))
}

/// DELETE /api/v1/identity-providers/{id} — and every identity at it.
#[utoipa::path(
    delete,
    path = "/api/v1/identity-providers/{id}",
    tag = "identity_providers",
    operation_id = "identity_providers_delete",
    params(("id" = String, Path)),
    responses(
        (status = 204, description = "Deleted"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn delete(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    crate::auth::check_permission(
        &identity,
        Permission::IdentityProviderManage,
        state.deployment_mode,
    )?;
    let provider = tenant_provider(&state, &tenant.0, &id).await?;
    if provider.enabled {
        ensure_a_way_in(&state, &tenant.0, &provider.id).await?;
    }
    identity_store(&state)?
        .delete_identity_provider(&provider.id)
        .await?;
    tracing::info!(provider = %provider.id, by = %identity.actor(), "Identity provider deleted");
    Ok(StatusCode::NO_CONTENT)
}

/// What a provider's check found.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ProviderCheck {
    /// The discovery document and keys were fetched.
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorization_endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// POST /api/v1/identity-providers/{id}/check — fetch the provider's
/// discovery document and keys, as a sign-in would.
#[utoipa::path(
    post,
    path = "/api/v1/identity-providers/{id}/check",
    tag = "identity_providers",
    operation_id = "identity_providers_check",
    params(("id" = String, Path)),
    responses(
        (status = 200, description = "What the check found", body = ProviderCheck),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn check_provider(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
) -> Result<Json<ProviderCheck>, ApiError> {
    crate::auth::check_permission(
        &identity,
        Permission::IdentityProviderManage,
        state.deployment_mode,
    )?;
    let provider = tenant_provider(&state, &tenant.0, &id).await?;
    let sso = sso(&state)?;
    Ok(Json(
        match sso.client.discovery(&provider.settings.issuer).await {
            Ok(d) => ProviderCheck {
                ok: true,
                authorization_endpoint: Some(d.authorization_endpoint),
                error: None,
            },
            Err(e @ (OidcError::Provider(_) | OidcError::Refused(_) | OidcError::BadToken(_))) => {
                ProviderCheck {
                    ok: false,
                    authorization_endpoint: None,
                    error: Some(e.to_string()),
                }
            }
        },
    ))
}

/// A user's account at one of the tenant's providers.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct UserIdentityResponse {
    pub provider_id: String,
    pub subject: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(with = "crate::time::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub created_at: SystemTime,
    #[serde(
        with = "crate::time::rfc3339_opt",
        skip_serializing_if = "Option::is_none"
    )]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub last_sign_in_at: Option<SystemTime>,
}

async fn tenant_user(
    state: &ApiState,
    tenant: &fcp_core::TenantId,
    username: &str,
) -> Result<User, ApiError> {
    storage(state)?
        .get_user_in_tenant(username, tenant)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("User '{username}' not found")))
}

/// GET /api/v1/users/{username}/identities
#[utoipa::path(
    get,
    path = "/api/v1/users/{username}/identities",
    tag = "users",
    operation_id = "users_list_identities",
    params(("username" = String, Path), crate::page::PageQuery),
    responses(
        (status = 200, description = "OK", body = crate::page::Page<UserIdentityResponse>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_identities(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(username): Path<String>,
    Query(page): Query<crate::page::PageQuery>,
) -> Result<Json<crate::page::Page<UserIdentityResponse>>, ApiError> {
    crate::auth::check_permission(&identity, Permission::UserRead, state.deployment_mode)?;
    let user = tenant_user(&state, &tenant.0, &username).await?;
    let identities = identity_store(&state)?
        .list_user_identities(&user.id)
        .await?;
    let items: Vec<UserIdentityResponse> = identities
        .into_iter()
        .map(|i| UserIdentityResponse {
            provider_id: i.provider_id.to_string(),
            subject: i.subject,
            email: i.email,
            created_at: i.created_at,
            last_sign_in_at: i.last_sign_in_at,
        })
        .collect();
    Ok(Json(crate::page::paginate(
        items,
        &page,
        crate::page::Order::Ascending,
        |i| format!("{}\u{0}{}", i.provider_id, i.subject),
    )?))
}

/// DELETE /api/v1/users/{username}/identities/{provider_id} — unlink a
/// user from their account at a provider; the next sign-in there links
/// or creates afresh, as the provider's settings say.
#[utoipa::path(
    delete,
    path = "/api/v1/users/{username}/identities/{provider_id}",
    tag = "users",
    operation_id = "users_unlink_identity",
    params(("username" = String, Path), ("provider_id" = String, Path)),
    responses(
        (status = 204, description = "Unlinked"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn unlink_identity(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path((username, provider_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    crate::auth::check_permission(&identity, Permission::UserUpdate, state.deployment_mode)?;
    let user = tenant_user(&state, &tenant.0, &username).await?;
    let store = identity_store(&state)?;
    let mut removed = false;
    for i in store.list_user_identities(&user.id).await? {
        if i.provider_id.to_string() == provider_id {
            removed |= store
                .delete_user_identity(&i.provider_id, &i.subject)
                .await?;
        }
    }
    if !removed {
        return Err(ApiError::NotFound("No such identity".to_string()));
    }
    Ok(StatusCode::NO_CONTENT)
}
