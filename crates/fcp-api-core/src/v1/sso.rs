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

/// The cookie that ties a provider's answer to the browser that began the
/// sign-in: random, HttpOnly, gone after ten minutes.
struct CookieSpec {
    name: &'static str,
    path: &'static str,
    /// Its answer comes back as a cross-site POST (SAML's HTTP-POST
    /// binding), which a `SameSite=Lax` cookie does not ride: `None`, and
    /// so `Secure`, over HTTPS. OIDC's answer is a top-level GET: `Lax`.
    cross_site_post: bool,
}

const OIDC_COOKIE: &CookieSpec = &CookieSpec {
    name: "fcp_sso",
    path: "/api/v1/auth/sso",
    cross_site_post: false,
};
const SAML_COOKIE: &CookieSpec = &CookieSpec {
    name: "fcp_saml",
    path: "/api/v1/auth/saml",
    cross_site_post: true,
};

impl CookieSpec {
    fn header(&self, value: &str, max_age: u64, public_url: &str) -> HeaderValue {
        let https = public_url.starts_with("https://");
        let same_site = if self.cross_site_post && https {
            "None"
        } else {
            "Lax"
        };
        let secure = if https { "; Secure" } else { "" };
        HeaderValue::from_str(&format!(
            "{}={value}; Path={}; Max-Age={max_age}; HttpOnly; SameSite={same_site}{secure}",
            self.name, self.path
        ))
        .expect("base64url is a header value")
    }

    fn set(&self, value: &str, public_url: &str) -> HeaderValue {
        self.header(
            value,
            fcp_core::identity::SSO_LOGIN_TTL.as_secs(),
            public_url,
        )
    }

    fn clear(&self, public_url: &str) -> HeaderValue {
        self.header("", 0, public_url)
    }
}

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
    let begun = Begun::new();
    // OIDC keeps the nonce and the PKCE verifier; SAML the id of its
    // AuthnRequest, which the response must answer.
    let (url, nonce, code_verifier, cookie) = match provider.kind {
        ProviderKind::Oidc => {
            let discovery = sso
                .client
                .discovery(&provider.settings.issuer)
                .await
                .map_err(|e| ApiError::ServiceUnavailable(e.to_string()))?;
            let url = sso
                .client
                .authorize_url(&discovery, &provider.settings, &sso.redirect_uri(), &begun)
                .map_err(|e| ApiError::ServiceUnavailable(e.to_string()))?;
            (url, begun.nonce, begun.code_verifier, OIDC_COOKIE)
        }
        ProviderKind::Saml => {
            let endpoints = crate::saml::Endpoints::new(&sso.public_url, &provider);
            let (url, request_id) = crate::saml::begin(&provider, &endpoints, &begun.state)
                .map_err(|e| ApiError::ServiceUnavailable(e.to_string()))?;
            (url, request_id, String::new(), SAML_COOKIE)
        }
    };
    store
        .create_sso_login(&SsoLogin {
            state: begun.state.clone(),
            provider_id: provider.id,
            tenant_id: provider.tenant_id,
            nonce,
            code_verifier,
            created_at: SystemTime::now(),
        })
        .await?;
    let mut response = Redirect::to(&url).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        cookie.set(&begun.state, &sso.public_url),
    );
    Ok(response)
}

/// What a SAML provider posts back (the HTTP-POST binding).
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct SamlPost {
    #[serde(rename = "SAMLResponse")]
    pub saml_response: String,
    #[serde(rename = "RelayState", default)]
    pub relay_state: Option<String>,
}

/// POST /api/v1/auth/saml/{id}/acs — a SAML provider's answer: to the
/// login page with a one-time code, or with the reason it failed.
#[utoipa::path(
    post,
    path = "/api/v1/auth/saml/{id}/acs",
    tag = "auth_session",
    operation_id = "auth_session_saml_acs",
    params(("id" = String, Path, description = "The identity provider")),
    request_body(content = SamlPost, content_type = "application/x-www-form-urlencoded"),
    responses(
        (status = 302, description = "To the login page, with `sso_code` or `sso_error`"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn saml_acs(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    axum::Form(post): axum::Form<SamlPost>,
) -> Result<Response, ApiError> {
    let sso = sso(&state)?.clone();
    let landing = |param: &str, value: &str| landing(&sso, SAML_COOKIE, param, value);

    // A sign-in this browser began: IdP-initiated sign-in is not taken.
    let Some(sent) = post.relay_state.as_deref().filter(|s| !s.is_empty()) else {
        return Ok(landing("sso_error", "bad_state"));
    };
    if cookie_value(&headers, SAML_COOKIE.name).as_deref() != Some(sent) {
        return Ok(landing("sso_error", "bad_state"));
    }
    let store = identity_store(&state)?;
    let Some(login) = store.take_sso_login(sent, SystemTime::now()).await? else {
        return Ok(landing("sso_error", "expired"));
    };
    let Some(provider) = store
        .get_identity_provider(&login.provider_id)
        .await?
        .filter(|p| p.id.to_string() == id)
    else {
        return Ok(landing("sso_error", "expired"));
    };
    let fail = |reason: &str, username: Option<&str>| {
        refused_sign_in(&state, &headers, &provider, reason, username);
        landing("sso_error", reason)
    };
    if !provider.enabled || provider.kind != ProviderKind::Saml {
        return Ok(fail("provider_refused", None));
    }
    let endpoints = crate::saml::Endpoints::new(&sso.public_url, &provider);
    let accepted =
        match crate::saml::accept(&provider, &endpoints, &post.saml_response, &login.nonce) {
            Ok(accepted) => accepted,
            Err(e) => {
                tracing::warn!(provider = %provider.id, error = %e, "SAML sign-in failed");
                return Ok(fail(e.reason(), None));
            }
        };
    // The same signed assertion posted twice is a replay.
    if !store
        .remember_saml_assertion(&provider.id, &accepted.assertion_id, accepted.expires)
        .await?
    {
        tracing::warn!(provider = %provider.id, "A SAML assertion presented again");
        return Ok(fail("bad_assertion", None));
    }
    let mut claims = accepted.claims;
    // SAML has no `email_verified`; the tenant says whether its directory's
    // addresses are its people's own.
    if provider
        .settings
        .saml
        .as_ref()
        .is_some_and(|s| s.trust_email)
    {
        claims.insert(
            provider.settings.email_verified_claim.clone(),
            Value::Bool(true),
        );
    }
    let hint = provider
        .settings
        .sign_out_at_provider
        .then_some(accepted.logout_hint);
    match hand_off(&state, &store, &provider, &claims, hint).await? {
        Ok(handoff) => Ok(landing("sso_code", &handoff)),
        Err(Refusal(reason, username)) => Ok(fail(reason, username.as_deref())),
    }
}

/// What a SAML provider posts to ask for a logout, or to answer FCP's
/// (the HTTP-POST binding).
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct SamlLogoutPost {
    #[serde(rename = "SAMLRequest", default)]
    pub saml_request: Option<String>,
    /// The provider's answer to a logout FCP asked for.
    #[serde(rename = "SAMLResponse", default)]
    pub saml_response: Option<String>,
    #[serde(rename = "RelayState", default)]
    pub relay_state: Option<String>,
}

/// The SAML provider a logout names, enabled or not: a logout is honoured
/// either way.
async fn saml_provider(state: &ApiState, id: &str) -> Result<IdentityProvider, ApiError> {
    let not_found = || ApiError::NotFound("No such identity provider".to_string());
    let id: IdentityProviderId = id.parse().map_err(|_| not_found())?;
    identity_store(state)?
        .get_identity_provider(&id)
        .await?
        .filter(|p| p.kind == ProviderKind::Saml)
        .ok_or_else(not_found)
}

/// End what a verified logout request names, and answer the provider.
async fn saml_logout(
    state: &ApiState,
    sso: &SsoRuntime,
    provider: &IdentityProvider,
    asked: crate::saml::LogoutAsked,
    relay_state: Option<&str>,
) -> Result<Response, ApiError> {
    let user = match &asked.name_id {
        Some(name_id) => identity_store(state)?
            .get_user_identity(&provider.id, name_id)
            .await?
            .map(|i| i.user_id),
        None => None,
    };
    let mut ended = 0;
    if user.is_some() || asked.session_index.is_some() {
        if let Some(sessions) = state
            .storage
            .as_ref()
            .and_then(|s| s.login_session_store().cloned())
        {
            ended = sessions
                .revoke_idp_sessions(
                    &provider.id.to_string(),
                    user.as_ref(),
                    asked.session_index.as_deref(),
                    "signed out at the identity provider",
                    SystemTime::now(),
                )
                .await?;
        }
    }
    tracing::info!(provider = %provider.id, ended, "SAML single logout");
    let endpoints = crate::saml::Endpoints::new(&sso.public_url, provider);
    match crate::saml::logout_response(provider, &endpoints, &asked.request_id, relay_state) {
        Ok(Some(url)) => Ok(no_store_redirect(&url)),
        // A provider without a logout address of its own gets a page.
        Ok(None) => Ok(no_store_page("Signed out.")),
        Err(e) => Err(ApiError::ServiceUnavailable(e.to_string())),
    }
}

/// The provider's answer to a logout FCP asked for: FCP's session ended
/// before the browser left, so whatever the answer says, the browser goes
/// to the login page.
fn signed_out_at_provider(sso: &SsoRuntime, provider: &IdentityProvider) -> Response {
    tracing::info!(provider = %provider.id, "Back from signing out at the SAML provider");
    no_store_redirect(&format!("{}/login", sso.public_url))
}

fn no_store_redirect(url: &str) -> Response {
    let mut response = Redirect::to(url).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

fn no_store_page(text: &'static str) -> Response {
    let mut response = text.into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// GET /api/v1/auth/saml/{id}/slo — a SAML provider's logout request
/// (HTTP-Redirect), its signature over the query string as it came.
#[utoipa::path(
    get,
    path = "/api/v1/auth/saml/{id}/slo",
    tag = "auth_session",
    operation_id = "auth_session_saml_slo",
    params(("id" = String, Path, description = "The identity provider")),
    responses(
        (status = 303, description = "Back to the provider with FCP's signed LogoutResponse, or to the login page after the provider answers FCP's own logout"),
        (status = 200, description = "Signed out, where the provider has no logout address"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn saml_slo_redirect(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    axum::extract::RawQuery(query): axum::extract::RawQuery,
) -> Result<Response, ApiError> {
    let sso = sso(&state)?.clone();
    let provider = saml_provider(&state, &id).await?;
    let endpoints = crate::saml::Endpoints::new(&sso.public_url, &provider);
    let query = query.unwrap_or_default();
    let has = |name: &str| query.split('&').any(|p| p.starts_with(&format!("{name}=")));
    if has("SAMLResponse") && !has("SAMLRequest") {
        return Ok(signed_out_at_provider(&sso, &provider));
    }
    let asked = crate::saml::logout_redirect(&provider, &endpoints, &query).map_err(|e| {
        tracing::warn!(provider = %provider.id, error = %e, "A SAML logout request refused");
        ApiError::BadRequest(e.to_string())
    })?;
    let relay = query
        .split('&')
        .find_map(|p| p.strip_prefix("RelayState="))
        .and_then(|r| urlencoding::decode(r).ok().map(|r| r.into_owned()));
    saml_logout(&state, &sso, &provider, asked, relay.as_deref()).await
}

/// POST /api/v1/auth/saml/{id}/slo — a SAML provider's logout request
/// (HTTP-POST), read only as far as it is signed.
#[utoipa::path(
    post,
    path = "/api/v1/auth/saml/{id}/slo",
    tag = "auth_session",
    operation_id = "auth_session_saml_slo_post",
    params(("id" = String, Path, description = "The identity provider")),
    request_body(content = SamlLogoutPost, content_type = "application/x-www-form-urlencoded"),
    responses(
        (status = 303, description = "Back to the provider with FCP's signed LogoutResponse, or to the login page after the provider answers FCP's own logout"),
        (status = 200, description = "Signed out, where the provider has no logout address"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn saml_slo_post(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    axum::Form(post): axum::Form<SamlLogoutPost>,
) -> Result<Response, ApiError> {
    let sso = sso(&state)?.clone();
    let provider = saml_provider(&state, &id).await?;
    let endpoints = crate::saml::Endpoints::new(&sso.public_url, &provider);
    let Some(saml_request) = post.saml_request.as_deref() else {
        if post.saml_response.is_some() {
            return Ok(signed_out_at_provider(&sso, &provider));
        }
        return Err(ApiError::BadRequest("No SAMLRequest".to_string()));
    };
    let asked = crate::saml::logout_post(&provider, &endpoints, saml_request).map_err(|e| {
        tracing::warn!(provider = %provider.id, error = %e, "A SAML logout request refused");
        ApiError::BadRequest(e.to_string())
    })?;
    saml_logout(&state, &sso, &provider, asked, post.relay_state.as_deref()).await
}

/// GET /api/v1/auth/saml/{id} — the service provider's metadata, for the
/// identity provider's administrator to import. Its address is also the
/// service provider's entity id.
#[utoipa::path(
    get,
    path = "/api/v1/auth/saml/{id}",
    tag = "auth_session",
    operation_id = "auth_session_saml_metadata",
    params(("id" = String, Path, description = "The identity provider")),
    responses(
        (status = 200, description = "SAML metadata", content_type = "application/samlmetadata+xml", body = String),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn saml_metadata(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let sso = sso(&state)?.clone();
    let not_found = || ApiError::NotFound("No such identity provider".to_string());
    let id: IdentityProviderId = id.parse().map_err(|_| not_found())?;
    let provider = identity_store(&state)?
        .get_identity_provider(&id)
        .await?
        .filter(|p| p.kind == ProviderKind::Saml)
        .ok_or_else(not_found)?;
    let endpoints = crate::saml::Endpoints::new(&sso.public_url, &provider);
    let xml = crate::saml::sp_metadata(&provider, &endpoints)
        .map_err(|e| ApiError::ServiceUnavailable(e.to_string()))?;
    let mut response = xml.into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/samlmetadata+xml"),
    );
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

fn cookie_value(headers: &HeaderMap, cookie: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .find_map(|pair| {
            let (name, value) = pair.trim().split_once('=')?;
            (name == cookie).then(|| value.to_string())
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
    let landing = |param: &str, value: &str| landing(&sso, OIDC_COOKIE, param, value);

    // The state must be the one this browser began with, and still waiting.
    let Some(sent) = q.state.as_deref().filter(|s| !s.is_empty()) else {
        return Ok(landing("sso_error", "bad_state"));
    };
    if cookie_value(&headers, OIDC_COOKIE.name).as_deref() != Some(sent) {
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
        refused_sign_in(&state, &headers, &provider, reason, username);
        landing("sso_error", reason)
    };
    if let Some(error) = q.error.as_deref() {
        tracing::info!(provider = %provider.id, error, "The identity provider refused the sign-in");
        return Ok(fail("provider_refused", None));
    }
    let (Some(code), true, ProviderKind::Oidc) =
        (q.code.as_deref(), provider.enabled, provider.kind)
    else {
        return Ok(fail("provider_refused", None));
    };

    let (claims, id_token) = match sso
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
        Ok(signed_in) => signed_in,
        Err(e) => {
            tracing::warn!(provider = %provider.id, error = %e, "Single sign-on failed");
            return Ok(fail(e.reason(), None));
        }
    };
    let hint = provider.settings.sign_out_at_provider.then_some(id_token);
    match hand_off(&state, &store, &provider, &claims, hint).await? {
        Ok(handoff) => Ok(landing("sso_code", &handoff)),
        Err(Refusal(reason, username)) => Ok(fail(reason, username.as_deref())),
    }
}

/// Where the browser lands after a provider: the login page with the code
/// or the reason, and the sign-in's cookie cleared.
fn landing(sso: &SsoRuntime, cookie: &CookieSpec, param: &str, value: &str) -> Response {
    let mut response = Redirect::to(&format!(
        "{}/login?{param}={}",
        sso.public_url,
        urlencoding::encode(value)
    ))
    .into_response();
    response
        .headers_mut()
        .insert(header::SET_COOKIE, cookie.clear(&sso.public_url));
    response
}

/// A refused sign-in, published and logged.
fn refused_sign_in(
    state: &ApiState,
    headers: &HeaderMap,
    provider: &IdentityProvider,
    reason: &str,
    username: Option<&str>,
) {
    publish(
        state,
        fcp_events::UserEvent::SignInFailed {
            username: username.unwrap_or_default().to_string(),
            tenant_id: Some(provider.tenant_id),
            reason: reason.to_string(),
            source_ip: crate::audit::source_ip(headers),
            timestamp: SystemTime::now(),
        },
    );
    tracing::warn!(provider = %provider.id, tenant_id = %provider.tenant_id, reason, "Single sign-on refused");
}

/// A verified sign-in's user, and the one-time code the login page
/// exchanges for their session; or why there is no user to be.
async fn hand_off(
    state: &ApiState,
    store: &Arc<dyn IdentityStore>,
    provider: &IdentityProvider,
    claims: &Map<String, Value>,
    logout_hint: Option<String>,
) -> Result<Result<String, Refusal>, ApiError> {
    let user = match resolve_user(state, provider, claims).await {
        Ok(user) => user,
        Err(refusal) => return Ok(Err(refusal)),
    };
    let handoff = crate::oidc::random_token();
    store
        .put_sso_handoff(
            &fcp_core::identity::SsoHandoff {
                code_hash: hash(&handoff),
                user_id: user.id,
                provider_id: provider.id,
                idp_sid: claim_str(claims, "sid"),
                idp_logout_hint: logout_hint,
            },
            SystemTime::now(),
        )
        .await?;
    Ok(Ok(handoff))
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

    // A tenant whose users come from SCIM (`docs/SCIM.md`): nobody is made
    // here, and only those SCIM made sign in.
    let scim = {
        use fcp_core::TenantStore;
        storage
            .get_tenant(&provider.tenant_id)
            .await
            .map_err(|_| refuse("internal"))?
            .is_some_and(|t| t.sign_in.scim)
    };
    if scim && user.as_ref().is_some_and(|u| !u.scim_managed) {
        return Err(refuse("not_provisioned"));
    }

    let mut user = match user {
        Some(user) => user,
        None if provider.settings.jit && !at_site && !scim => {
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
                || crate::v1::users::sip_name_taken(storage, &provider.tenant_id, &username, None)
                    .await
                    .map_err(|_| refuse("internal"))?
            {
                return Err(refuse("username_taken"));
            }
            if crate::v1::users::ensure_room_for_a_user(state, &provider.tenant_id)
                .await
                .is_err()
            {
                return Err(refuse("tenant_full"));
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
    // The tenant's groups that follow the provider's (docs/USERS.md §5):
    // the user's membership is the provider's word, when it says.
    if let (Some(groups), false) = (groups.as_deref(), at_site) {
        crate::v1::groups::follow_idp_groups(state, &user, groups)
            .await
            .map_err(|_| refuse("internal"))?;
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
    let method = match identity_store(&state)?
        .get_identity_provider(&handoff.provider_id)
        .await?
        .map(|p| p.kind)
    {
        Some(ProviderKind::Saml) => session::method::SAML,
        _ => session::method::OIDC,
    };
    let idp = session::IdpSession {
        provider_id: handoff.provider_id.to_string(),
        sid: handoff.idp_sid,
        logout_hint: handoff.idp_logout_hint,
    };
    let mut response =
        complete_sign_in_through(&state, &headers, user, method, None, Some(idp)).await?;
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
        .filter(|p| p.kind == ProviderKind::Oidc)
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
    /// The address to register with the provider as the redirect URI —
    /// for SAML, the assertion consumer service; absent where `[api.sso]
    /// public_url` is not set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redirect_uri: Option<String>,
    /// OIDC: the address to register as its back-channel logout URI.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backchannel_logout_uri: Option<String>,
    /// SAML: this service provider's entity id, which is also where its
    /// metadata is served for the identity provider to import.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sp_entity_id: Option<String>,
    /// SAML: FCP's single logout address (both bindings).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sp_logout_url: Option<String>,
    /// SAML: whether FCP has its own key for this provider, to sign its
    /// requests and be encrypted to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sp_key: Option<bool>,
    #[serde(with = "crate::time::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub created_at: SystemTime,
    #[serde(with = "crate::time::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: SystemTime,
}

impl IdentityProviderResponse {
    fn from(state: &ApiState, p: IdentityProvider) -> Self {
        let saml = state
            .sso
            .as_ref()
            .filter(|_| p.kind == ProviderKind::Saml)
            .map(|s| crate::saml::Endpoints::new(&s.public_url, &p));
        let redirect_uri = match (&saml, &state.sso) {
            (Some(e), _) => Some(e.acs_url.clone()),
            (None, Some(s)) => Some(s.redirect_uri()),
            (None, None) => None,
        };
        Self {
            id: p.id.to_string(),
            tenant_id: p.tenant_id.to_string(),
            name: p.name,
            kind: p.kind,
            enabled: p.enabled,
            settings: p.settings,
            has_client_secret: p.client_secret.is_some(),
            redirect_uri,
            backchannel_logout_uri: state
                .sso
                .as_ref()
                .filter(|_| p.kind == ProviderKind::Oidc)
                .map(|s| {
                    format!(
                        "{}/api/v1/auth/sso/{}/backchannel-logout",
                        s.public_url, p.id
                    )
                }),
            sp_logout_url: saml.as_ref().map(|e| e.slo_url.clone()),
            sp_entity_id: saml.map(|e| e.entity_id),
            sp_key: (p.kind == ProviderKind::Saml).then_some(p.client_secret.is_some()),
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
    /// `oidc` (the default) or `saml`.
    #[serde(default)]
    #[schema(value_type = Option<String>)]
    pub kind: ProviderKind,
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
    if let Some(problem) = req.settings.problems(req.kind) {
        return Err(ApiError::BadRequest(problem));
    }
    if req.kind == ProviderKind::Saml {
        let xml = req
            .settings
            .saml
            .as_ref()
            .map(|s| s.idp_metadata_xml.as_str())
            .unwrap_or_default();
        return match crate::saml::summarize(xml) {
            Ok(_) => Ok(()),
            // A build without SAML keeps the provider for one with it.
            Err(crate::saml::SamlError::Unavailable) => Ok(()),
            Err(e) => Err(ApiError::BadRequest(e.to_string())),
        };
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

/// A SAML provider's metadata fetched from its URL, where one is given,
/// through the same guard as an OIDC issuer.
async fn fetch_metadata(state: &ApiState, settings: &mut ProviderSettings) -> Result<(), ApiError> {
    let Some(saml) = settings.saml.as_mut() else {
        return Ok(());
    };
    let Some(url) = saml
        .idp_metadata_url
        .as_deref()
        .map(str::trim)
        .filter(|u| !u.is_empty())
        .map(str::to_string)
    else {
        return Ok(());
    };
    let fetched = match state.sso.as_ref() {
        Some(sso) => sso.client.fetch_text(&url).await,
        None => crate::oidc::OidcClient::new(false).fetch_text(&url).await,
    };
    saml.idp_metadata_xml =
        fetched.map_err(|e| ApiError::BadRequest(format!("saml.idp_metadata_url: {e}")))?;
    Ok(())
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

/// Give a SAML provider a key of FCP's own: its private half the
/// provider's sealed secret, its certificate in FCP's metadata. A build
/// without SAML gives none (and signs nobody in through it).
fn give_sp_key(provider: &mut IdentityProvider) -> Result<(), ApiError> {
    let key = match crate::saml::generate_sp_key() {
        Ok(key) => key,
        Err(crate::saml::SamlError::Unavailable) => return Ok(()),
        Err(e) => return Err(ApiError::Internal(e.to_string())),
    };
    if let Some(saml) = provider.settings.saml.as_mut() {
        saml.sp_certificate = Some(key.certificate);
        provider.client_secret = Some(key.private_key);
    }
    Ok(())
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
    let mut req = req;
    if let Some(saml) = req.settings.saml.as_mut() {
        // The server makes FCP's key; a request's certificate is not taken.
        saml.sp_certificate = None;
    }
    if req.kind == ProviderKind::Saml {
        req.client_secret = None;
    }
    fetch_metadata(&state, &mut req.settings).await?;
    check(&state, &req)?;
    let now = SystemTime::now();
    let mut provider = IdentityProvider {
        id: IdentityProviderId::new(),
        tenant_id: tenant.0,
        name: req.name.trim().to_string(),
        kind: req.kind,
        enabled: req.enabled,
        settings: req.settings,
        client_secret: req.client_secret.filter(|s| !s.is_empty()),
        created_at: now,
        updated_at: now,
    };
    if provider.kind == ProviderKind::Saml {
        give_sp_key(&mut provider)?;
    }
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
    let mut req = req;
    fetch_metadata(&state, &mut req.settings).await?;
    check(&state, &req)?;
    let mut provider = tenant_provider(&state, &tenant.0, &id).await?;
    if provider.enabled && !req.enabled {
        ensure_a_way_in(&state, &tenant.0, &provider.id).await?;
    }
    if req.kind != provider.kind {
        return Err(ApiError::BadRequest(
            "a provider's protocol cannot change; add another provider".to_string(),
        ));
    }
    provider.name = req.name.trim().to_string();
    provider.enabled = req.enabled;
    let sp_certificate = provider
        .settings
        .saml
        .as_ref()
        .and_then(|s| s.sp_certificate.clone());
    provider.settings = req.settings;
    match provider.kind {
        // FCP's own key for a SAML provider is the server's: kept.
        ProviderKind::Saml => {
            if let Some(saml) = provider.settings.saml.as_mut() {
                saml.sp_certificate = sp_certificate;
            }
            if provider.client_secret.is_none() {
                give_sp_key(&mut provider)?;
            }
        }
        ProviderKind::Oidc => match req.client_secret {
            Some(s) if s.is_empty() => provider.client_secret = None,
            Some(s) => provider.client_secret = Some(s),
            None => {}
        },
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

/// POST /api/v1/identity-providers/{id}/sp-key — a new key of FCP's own
/// for a SAML provider. The provider must then import FCP's metadata
/// again: until it does, it refuses what FCP signs with the new key.
#[utoipa::path(
    post,
    path = "/api/v1/identity-providers/{id}/sp-key",
    tag = "identity_providers",
    operation_id = "identity_providers_rotate_sp_key",
    params(("id" = String, Path)),
    responses(
        (status = 200, description = "The provider, with FCP's new certificate", body = IdentityProviderResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn rotate_sp_key(
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
    let mut provider = tenant_provider(&state, &tenant.0, &id).await?;
    if provider.kind != ProviderKind::Saml {
        return Err(ApiError::BadRequest(
            "only a SAML provider has a key of FCP's".into(),
        ));
    }
    let key =
        crate::saml::generate_sp_key().map_err(|e| ApiError::ServiceUnavailable(e.to_string()))?;
    if let Some(saml) = provider.settings.saml.as_mut() {
        saml.sp_certificate = Some(key.certificate);
    }
    provider.client_secret = Some(key.private_key);
    provider.updated_at = SystemTime::now();
    identity_store(&state)?
        .put_identity_provider(&provider)
        .await?;
    tracing::info!(provider = %provider.id, by = %identity.actor(), "FCP's SAML key rotated");
    Ok(Json(IdentityProviderResponse::from(&state, provider)))
}

/// POST /api/v1/identity-providers/{id}/refresh-metadata — fetch a SAML
/// provider's metadata again from its URL: the certificate it signs with
/// now, after it rotates one.
#[utoipa::path(
    post,
    path = "/api/v1/identity-providers/{id}/refresh-metadata",
    tag = "identity_providers",
    operation_id = "identity_providers_refresh_metadata",
    params(("id" = String, Path)),
    responses(
        (status = 200, description = "The provider, its metadata fetched", body = IdentityProviderResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn refresh_metadata(
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
    let mut provider = tenant_provider(&state, &tenant.0, &id).await?;
    let has_url = provider
        .settings
        .saml
        .as_ref()
        .and_then(|s| s.idp_metadata_url.as_deref())
        .is_some_and(|u| !u.trim().is_empty());
    if !has_url {
        return Err(ApiError::BadRequest(
            "the provider has no metadata URL to fetch".into(),
        ));
    }
    fetch_metadata(&state, &mut provider.settings).await?;
    let xml = provider
        .settings
        .saml
        .as_ref()
        .map(|s| s.idp_metadata_xml.clone())
        .unwrap_or_default();
    match crate::saml::summarize(&xml) {
        Ok(_) | Err(crate::saml::SamlError::Unavailable) => {}
        Err(e) => return Err(ApiError::BadRequest(e.to_string())),
    }
    provider.updated_at = SystemTime::now();
    identity_store(&state)?
        .put_identity_provider(&provider)
        .await?;
    Ok(Json(IdentityProviderResponse::from(&state, provider)))
}

/// What a provider's check found.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ProviderCheck {
    /// OIDC: the discovery document and keys were fetched; SAML: the
    /// metadata names a single sign-on address and a signing certificate.
    pub ok: bool,
    /// Where people are sent to sign in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorization_endpoint: Option<String>,
    /// SAML: the provider's entity id, its logout address, and how many
    /// certificates it signs with.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idp_entity_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idp_logout_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signing_certificates: Option<usize>,
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
    if provider.kind == ProviderKind::Saml {
        let xml = provider
            .settings
            .saml
            .as_ref()
            .map(|s| s.idp_metadata_xml.as_str())
            .unwrap_or_default();
        return Ok(Json(match crate::saml::summarize(xml) {
            Ok(summary) => ProviderCheck {
                ok: true,
                authorization_endpoint: Some(summary.sso_url),
                idp_entity_id: Some(summary.entity_id),
                idp_logout_url: summary.slo_url,
                signing_certificates: Some(summary.signing_certificates),
                error: None,
            },
            Err(e) => ProviderCheck {
                ok: false,
                authorization_endpoint: None,
                idp_entity_id: None,
                idp_logout_url: None,
                signing_certificates: None,
                error: Some(e.to_string()),
            },
        }));
    }
    let sso = sso(&state)?;
    Ok(Json(
        match sso.client.discovery(&provider.settings.issuer).await {
            Ok(d) => ProviderCheck {
                ok: true,
                authorization_endpoint: Some(d.authorization_endpoint),
                idp_entity_id: None,
                idp_logout_url: None,
                signing_certificates: None,
                error: None,
            },
            Err(e @ (OidcError::Provider(_) | OidcError::Refused(_) | OidcError::BadToken(_))) => {
                ProviderCheck {
                    ok: false,
                    authorization_endpoint: None,
                    idp_entity_id: None,
                    idp_logout_url: None,
                    signing_certificates: None,
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

/// Where a signed-out session's browser goes to sign out at its provider
/// too (`docs/SSO.md` §5), when the provider says it should: OIDC's
/// `end_session_endpoint`, or the SAML provider's logout address with
/// FCP's signed `LogoutRequest`. `None` when it should not, or cannot.
pub(crate) async fn provider_sign_out_url(
    state: &ApiState,
    idp: &session::IdpSession,
) -> Option<String> {
    let sso = state.sso.as_ref()?;
    let id: IdentityProviderId = idp.provider_id.parse().ok()?;
    let provider = identity_store(state)
        .ok()?
        .get_identity_provider(&id)
        .await
        .ok()
        .flatten()?;
    if !provider.settings.sign_out_at_provider {
        return None;
    }
    let url = match provider.kind {
        ProviderKind::Oidc => sso
            .client
            .end_session_url(
                &provider.settings,
                idp.logout_hint.as_deref(),
                &format!("{}/login", sso.public_url),
            )
            .await
            .map_err(|e| e.to_string()),
        ProviderKind::Saml => {
            // FCP names whom to sign out; a session begun before the
            // setting was turned on kept nobody to name.
            let hint = idp.logout_hint.as_deref()?;
            let endpoints = crate::saml::Endpoints::new(&sso.public_url, &provider);
            crate::saml::logout_request(&provider, &endpoints, hint, idp.sid.as_deref())
                .map_err(|e| e.to_string())
        }
    };
    match url {
        Ok(url) => url,
        Err(e) => {
            tracing::warn!(provider = %provider.id, error = %e, "Cannot sign out at the identity provider");
            None
        }
    }
}

/// How often SAML providers' metadata is fetched again from its URL.
const METADATA_REFRESH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(24 * 3600);

/// Fetch every SAML provider's metadata again from its URL, keeping what
/// it had when the fetch fails or the new metadata is unusable: a
/// provider that rotates its certificate is followed without an
/// administrator's refresh (`docs/SSO.md` §8). Returns how many changed.
pub async fn refresh_all_metadata(state: &ApiState) -> usize {
    let (Some(storage), Ok(store)) = (state.storage.as_ref(), identity_store(state)) else {
        return 0;
    };
    let tenants = match storage.list_tenants().await {
        Ok(tenants) => tenants,
        Err(e) => {
            tracing::warn!(error = %e, "SAML metadata refresh: failed to list tenants");
            return 0;
        }
    };
    let mut changed = 0;
    for tenant in tenants {
        let providers = match store.list_identity_providers(&tenant.id).await {
            Ok(providers) => providers,
            Err(e) => {
                tracing::warn!(tenant_id = %tenant.id, error = %e, "SAML metadata refresh: failed to list providers");
                continue;
            }
        };
        for mut provider in providers {
            let Some(before) = provider
                .settings
                .saml
                .as_ref()
                .filter(|s| {
                    s.idp_metadata_url
                        .as_deref()
                        .is_some_and(|u| !u.trim().is_empty())
                })
                .map(|s| s.idp_metadata_xml.clone())
            else {
                continue;
            };
            if let Err(e) = fetch_metadata(state, &mut provider.settings).await {
                tracing::warn!(provider = %provider.id, error = ?e, "SAML metadata refresh: the fetch failed; keeping what it had");
                continue;
            }
            let after = provider
                .settings
                .saml
                .as_ref()
                .map(|s| s.idp_metadata_xml.clone())
                .unwrap_or_default();
            if after == before {
                continue;
            }
            match crate::saml::summarize(&after) {
                Ok(_) | Err(crate::saml::SamlError::Unavailable) => {}
                Err(e) => {
                    tracing::warn!(provider = %provider.id, error = %e, "SAML metadata refresh: the new metadata is unusable; keeping what it had");
                    continue;
                }
            }
            provider.updated_at = SystemTime::now();
            match store.put_identity_provider(&provider).await {
                Ok(()) => {
                    tracing::info!(provider = %provider.id, tenant_id = %tenant.id, "SAML metadata refreshed: it changed");
                    changed += 1;
                }
                Err(e) => {
                    tracing::warn!(provider = %provider.id, error = %e, "SAML metadata refresh: failed to save")
                }
            }
        }
    }
    changed
}

/// [`refresh_all_metadata`] once a day, by one admin replica at a time,
/// fetching as `[api.sso]` allows. A site's admin takes its providers from
/// the cloud and refreshes none.
pub fn spawn_metadata_refresh(state: ApiState, sso: &fcp_core::ApiSsoConfig) {
    if state.managed_by.is_some() {
        return;
    }
    let state = state.with_sso_config(sso);
    tokio::spawn(async move {
        use fcp_core::storage::CdrStore;
        let mut interval = tokio::time::interval(METADATA_REFRESH_INTERVAL);
        loop {
            interval.tick().await;
            let Some(storage) = state.storage.clone() else {
                continue;
            };
            let lock = match storage.try_maintenance_lock("fcp:saml-metadata").await {
                Ok(Some(lock)) => lock,
                Ok(None) => continue,
                Err(e) => {
                    tracing::warn!(error = %e, "SAML metadata refresh: could not take the lock");
                    continue;
                }
            };
            refresh_all_metadata(&state).await;
            lock.release().await;
        }
    });
}
