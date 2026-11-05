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
use serde::{Deserialize, Serialize};
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

/// A line on the app (`docs/DEVICE_LINES.md`): an account it registers
/// with its own credentials and takes calls on.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SoftphoneLine {
    /// 1 for the first; the first is the user's own.
    pub index: u32,
    /// `own`, `extension`, `shared` or `on_behalf`.
    pub kind: String,
    /// The line itself, as `PUT /me/lines/{line}/label` names it.
    pub line: String,
    /// What the app registers the line as.
    pub aor: String,
    /// What it is called: the user's name, the administrator's, or
    /// `default_label`.
    pub label: String,
    pub default_label: String,
    pub shared: bool,
    pub appearances: u32,
    /// A shared line whose active calls the app may join (barge in).
    pub barge_in: bool,
    /// Whether calls to the line ring the app.
    pub ring: bool,
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
    /// Listed in the tenant's directory, and whether the user may change
    /// that themselves (`directory_listed` on `PUT /me/profile`).
    pub directory_listed: bool,
    pub may_change_directory_listing: bool,
    /// The app's lines, its own first; `account` is the first, kept for a
    /// release.
    pub lines: Vec<SoftphoneLine>,
    /// Whether the user may name their lines (the tenant's choice).
    pub may_rename_lines: bool,
    /// Whether the app may capture calls, until when, and how large a
    /// capture may be (`docs/CALL_DIAGNOSTICS.md` §3).
    pub capture: crate::v1::device_captures::SoftphoneCapture,
}

/// The app's session and device: what `/me/softphone` answers for.
pub(crate) async fn app_device(state: &ApiState, me: &Me) -> Result<(User, Device), ApiError> {
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
    let aor = |user_part: &str| format!("sip:{user_part}@{domain}");
    let lines: Vec<SoftphoneLine> = fcp_sip::device_lines::shown(storage, &device)
        .await
        .into_iter()
        .map(|l| SoftphoneLine {
            index: l.index,
            kind: l.kind.as_str().to_string(),
            line: l.key,
            // The app's own line is the user's address, as `account` is.
            aor: if l.kind == fcp_core::LineKind::Own {
                aor(user.sip_name())
            } else {
                aor(&l.aor_user)
            },
            label: l.label,
            default_label: l.default_label,
            shared: l.shared,
            appearances: l.appearances,
            barge_in: l.barge_in,
            ring: l.ring,
        })
        .collect();
    let may_rename_lines = storage
        .get_tenant(&user.tenant_id)
        .await?
        .is_some_and(|t| t.lines.users_may_rename);
    let body = SoftphoneConfig {
        account: SoftphoneAccount {
            aor: format!("sip:{}@{domain}", user.sip_name()),
            display_name: user.shown_name(),
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
        directory_listed: user.directory_listed,
        may_change_directory_listing: crate::v1::self_service::may_change_listing(storage, &user)
            .await?,
        lines,
        may_rename_lines,
        capture: crate::v1::device_captures::capture_permit(&state, &user, &device.id).await?,
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

/// A user's name for one of their lines.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LineLabelRequest {
    /// Empty clears it: the line shows the administrator's label again.
    pub label: String,
}

/// PUT /api/v1/me/lines/{line}/label — name one of your lines, on every
/// device that has it (`docs/DEVICE_LINES.md` §2). 403 unless the tenant
/// lets its users rename; 404 for a line none of your devices has.
#[utoipa::path(
    put,
    path = "/api/v1/me/lines/{line}/label",
    tag = "softphone",
    operation_id = "softphone_set_line_label",
    params(("line" = String, Path, description = "`own`, `extension:1005`, `shared:+15551234567`, `on_behalf:<user>`")),
    request_body = LineLabelRequest,
    responses(
        (status = 204, description = "Named"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn set_line_label(
    State(state): State<ApiState>,
    me: Me,
    axum::extract::Path(line): axum::extract::Path<String>,
    axum::Json(req): axum::Json<LineLabelRequest>,
) -> Result<Response, ApiError> {
    let storage = state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::Internal("Storage not configured".to_string()))?;
    let user = crate::v1::self_service::load_claims_user(storage, &me.0, &state).await?;
    let allowed = storage
        .get_tenant(&user.tenant_id)
        .await?
        .is_some_and(|t| t.lines.users_may_rename);
    if !allowed {
        return Err(ApiError::Forbidden(
            "Your organisation names your lines; ask your administrator".to_string(),
        ));
    }
    let label = req.label.trim();
    if label.len() > fcp_core::line_label::MAX_LABEL_LEN || label.chars().any(char::is_control) {
        return Err(ApiError::BadRequest(format!(
            "a line's name is at most {} characters, with no control characters",
            fcp_core::line_label::MAX_LABEL_LEN
        )));
    }
    let mut known = false;
    for device in storage.list_devices_by_user(&user.id).await? {
        if fcp_sip::device_lines::shown(storage, &device)
            .await
            .iter()
            .any(|l| l.key == line)
        {
            known = true;
            break;
        }
    }
    if !known {
        return Err(ApiError::NotFound(format!(
            "None of your devices has the line {line}"
        )));
    }
    let store = storage
        .line_label_store()
        .ok_or_else(|| ApiError::Internal("Line label store not configured".to_string()))?;
    store
        .set_line_label(&user.tenant_id, &user.id, &line, label)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(StatusCode::NO_CONTENT.into_response())
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
