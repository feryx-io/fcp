//! What the softphone is told (`docs/SOFTPHONE.md` §2.4, §2.6): its SIP
//! account and settings at `GET /me/softphone`, a new SIP password at
//! `POST /me/softphone/rotate-password`, and where FCP is at
//! `/.well-known/fcp-provisioning`.

use crate::{
    error::ApiError,
    state::ApiState,
    v1::auth_session::{session_store, Me},
};
use axum::{
    extract::State,
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use fcp_core::storage::{CredentialStore, DeviceStore};
use fcp_core::{Device, TenantStore, User};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// The SIP account the app registers with.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SoftphoneAccount {
    /// The address of record: the user in the tenant's domain.
    pub aor: String,
    pub display_name: Option<String>,
    /// The tenant's SIP domain.
    pub domain: String,
    /// The Request-URI of a REGISTER: `sip:<domain>`.
    pub registrar: String,
    /// Where every request goes when the domain is not the server's
    /// address: `sip:<server>:<port>;transport=<transport>`.
    pub outbound_proxy: Option<String>,
    /// `udp`, `tcp` or `tls`.
    pub transport: String,
    /// The realm the password is hashed in.
    pub realm: String,
    /// The device's own SIP username; the password came with the sign-in.
    pub sip_username: String,
    /// Seconds a registration is asked for.
    pub register_expires: u32,
}

/// The media the app offers.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SoftphoneMedia {
    /// In order of preference, as SDP names them.
    pub codecs: Vec<String>,
    /// `optional` over TLS (SDES-SRTP offered, plain RTP taken), else `off`.
    pub srtp: String,
    /// `rfc2833`.
    pub dtmf: String,
}

/// Everything the app needs.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SoftphoneConfig {
    pub account: SoftphoneAccount,
    pub media: SoftphoneMedia,
    /// The user as FCP knows them.
    pub username: String,
    pub extension: Option<String>,
    /// The feature codes, by name (`park`, `voicemail`, `dnd_on`, …).
    pub feature_codes: BTreeMap<String, String>,
    /// Where this admin is, for the app's API calls.
    pub api_url: Option<String>,
    /// The device the app is.
    pub device_id: String,
    /// The user may record a call they are on
    /// (`/me/calls/{call_id}/recording/start`); the app shows its record
    /// button only then.
    pub may_record: bool,
}

/// The app's session and device: what `/me/softphone` answers for.
async fn app_device(state: &ApiState, me: &Me) -> Result<(User, Device), ApiError> {
    let not_app = || {
        ApiError::BadRequest(
            "This is the softphone's: sign in through the app (docs/APP_SIGN_IN.md)".to_string(),
        )
    };
    let sid =
        me.0.sid
            .as_deref()
            .and_then(|s| s.parse::<fcp_core::LoginSessionId>().ok())
            .ok_or_else(not_app)?;
    let row = session_store(state)
        .ok_or_else(not_app)?
        .get_login_session(&sid)
        .await?
        .filter(|s| s.app.is_some())
        .ok_or_else(not_app)?;
    let device_id = row.device_id.ok_or_else(not_app)?;
    let storage = state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::Internal("Storage not configured".to_string()))?;
    let user = storage
        .get_user_by_id(&row.user_id)
        .await?
        .ok_or_else(|| ApiError::Unauthorized("Sign in again".to_string()))?;
    let device = storage
        .get_device(&device_id)
        .await?
        .filter(|d| d.owner_user_id == Some(user.id))
        .ok_or_else(|| {
            ApiError::NotFound("The app's device has been removed: sign in again".to_string())
        })?;
    Ok((user, device))
}

/// GET /api/v1/me/softphone — the app's SIP account and settings, with an
/// ETag (`If-None-Match` answers 304).
#[utoipa::path(
    get,
    path = "/api/v1/me/softphone",
    tag = "softphone",
    operation_id = "softphone_get_config",
    responses(
        (status = 200, description = "OK", body = SoftphoneConfig),
        (status = 304, description = "Unchanged since the ETag given"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn get_config(
    State(state): State<ApiState>,
    headers: HeaderMap,
    me: Me,
) -> Result<Response, ApiError> {
    let (user, device) = app_device(&state, &me).await?;
    let storage = state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::Internal("Storage not configured".to_string()))?;
    let config = &state.provisioning;
    let server = config
        .sip_server
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| state.realm.clone());
    let transport = config.transport().to_string();
    let domain = storage
        .get_tenant(&user.tenant_id)
        .await?
        .and_then(|t| {
            t.sip_domains
                .iter()
                .find(|d| !d.trim().is_empty())
                .map(|d| d.trim().to_string())
        })
        .unwrap_or_else(|| server.clone());
    // The server as the outbound proxy when the domain does not name it,
    // as the phone templates do (`docs/PROVISIONING.md` §5).
    let outbound_proxy =
        (server != domain).then(|| format!("sip:{server}:{};transport={transport}", config.port()));
    // What this build can take, best first.
    let supported = fcp_media::codec::supported_codec_registry();
    let codecs: Vec<String> = ["opus", "G722", "PCMU", "PCMA"]
        .into_iter()
        .filter(|c| supported.contains_key(*c))
        .map(String::from)
        .collect();
    let feature_codes = serde_json::to_value(fcp_pbx::config::FeatureCodeConfig::default())
        .ok()
        .and_then(|v| v.as_object().cloned())
        .map(|map| {
            map.into_iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k, s.to_string())))
                .collect()
        })
        .unwrap_or_default();
    let body = SoftphoneConfig {
        account: SoftphoneAccount {
            aor: format!("sip:{}@{domain}", user.sip_name()),
            display_name: user.display_name.clone(),
            registrar: format!("sip:{domain}"),
            domain,
            outbound_proxy,
            transport: transport.clone(),
            realm: state.realm.clone(),
            sip_username: device.sip_username.clone().unwrap_or_default(),
            register_expires: 3600,
        },
        media: SoftphoneMedia {
            codecs,
            srtp: if transport == "tls" {
                "optional"
            } else {
                "off"
            }
            .to_string(),
            dtmf: "rfc2833".to_string(),
        },
        username: user.username.clone(),
        extension: user.extension.clone(),
        feature_codes,
        api_url: crate::v1::provisioning::public_base(&state, &headers)
            .map(|base| format!("{base}/api/v1")),
        device_id: device.id.to_string(),
        may_record: user.media_preferences.record_on_demand,
    };
    let json = serde_json::to_vec(&body).map_err(|e| ApiError::Internal(e.to_string()))?;
    let digest: String = Sha256::digest(&json)
        .iter()
        .take(16)
        .map(|b| format!("{b:02x}"))
        .collect();
    let etag = format!("\"{digest}\"");
    let matched = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(',').any(|t| t.trim() == etag));
    let mut response = if matched {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        ([(header::CONTENT_TYPE, "application/json")], json).into_response()
    };
    response.headers_mut().insert(
        header::ETAG,
        HeaderValue::from_str(&etag).expect("hex in quotes is a header value"),
    );
    Ok(response)
}

/// A new SIP password for the app's device.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SoftphonePassword {
    pub sip_username: String,
    /// Shown here once.
    pub sip_password: String,
    pub realm: String,
}

/// POST /api/v1/me/softphone/rotate-password — a new SIP password for the
/// app's device; the old one stops at once.
#[utoipa::path(
    post,
    path = "/api/v1/me/softphone/rotate-password",
    tag = "softphone",
    operation_id = "softphone_rotate_password",
    responses(
        (status = 200, description = "The new password", body = SoftphonePassword),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn rotate_password(
    State(state): State<ApiState>,
    me: Me,
) -> Result<Json<SoftphonePassword>, ApiError> {
    let (_, mut device) = app_device(&state, &me).await?;
    let password = crate::v1::app_auth::set_sip_password(&state, &mut device).await?;
    Ok(Json(SoftphonePassword {
        sip_username: device.sip_username.clone().unwrap_or_default(),
        sip_password: password,
        realm: state.realm.clone(),
    }))
}

/// Where FCP is, for an app finding it from an email domain.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct WellKnown {
    /// This admin, as browsers reach it.
    pub public_url: Option<String>,
    pub api_url: Option<String>,
    /// Where an app begins its sign-in.
    pub authorize_url: Option<String>,
    pub token_url: Option<String>,
    pub version: &'static str,
}

/// GET /.well-known/fcp-provisioning — where this FCP is and how an app
/// signs in (`docs/SOFTPHONE.md` §2.6). Public.
pub async fn well_known(State(state): State<ApiState>, headers: HeaderMap) -> Json<WellKnown> {
    let base = crate::v1::provisioning::public_base(&state, &headers);
    Json(WellKnown {
        api_url: base.as_ref().map(|b| format!("{b}/api/v1")),
        authorize_url: base
            .as_ref()
            .map(|b| format!("{b}/api/v1/auth/app/authorize")),
        token_url: base.as_ref().map(|b| format!("{b}/api/v1/auth/app/token")),
        public_url: base,
        version: env!("CARGO_PKG_VERSION"),
    })
}
