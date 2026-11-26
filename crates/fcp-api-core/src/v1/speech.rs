//! A tenant's speech settings (`docs/TENANT_SPEECH.md`, phase S2): the
//! provider, voice and language the TTS server speaks the tenant's words
//! with when a request names none. Read with `prompt_read`, changed with
//! `prompt_manage`: speech is what the tenant's prompts sound like.

use crate::auth::{AuthIdentity, TenantContext};
use crate::error::ApiError;
use crate::state::ApiState;
use axum::{extract::State, http::StatusCode, Extension, Json};
use fcp_core::permissions::Permission;
use fcp_core::speech::{SpeechSettingsStore, TenantSpeech};
use fcp_events::{PlatformEvent, SystemEvent};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::SystemTime;

fn store(state: &ApiState) -> Result<&Arc<dyn SpeechSettingsStore>, ApiError> {
    state
        .storage
        .as_ref()
        .and_then(|s| s.speech_store())
        .ok_or_else(|| ApiError::ServiceUnavailable("Speech settings are not kept here".into()))
}

/// The tenant's speech settings; nothing set is the platform's defaults.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SpeechSettingsResponse {
    pub tenant_id: String,
    /// One of the TTS server's providers; null is its default.
    pub provider: Option<String>,
    /// The voice where no language below names one; null is the provider's.
    pub voice: Option<String>,
    /// What a request without a language is spoken in; null is `en-US`.
    pub language: Option<String>,
    /// A voice per language (`es` or `es-ES`), over `voice`.
    pub voices: BTreeMap<String, String>,
    /// False until the tenant saved settings.
    pub configured: bool,
    #[serde(with = "crate::time::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: SystemTime,
}

impl SpeechSettingsResponse {
    fn of(s: TenantSpeech, configured: bool) -> Self {
        Self {
            tenant_id: s.tenant_id.to_string(),
            provider: s.provider,
            voice: s.voice,
            language: s.language,
            voices: s.voices,
            configured,
            updated_at: s.updated_at,
        }
    }
}

/// The settings to keep; null or absent is the platform's default.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct SpeechSettingsRequest {
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub voice: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub voices: BTreeMap<String, String>,
}

/// A name the TTS server might know: letters, digits, `-`, `_`, `.`, `:`.
fn checked(what: &str, value: Option<String>) -> Result<Option<String>, ApiError> {
    let Some(v) = value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
    else {
        return Ok(None);
    };
    let fine = v.len() <= 128
        && v.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'));
    if !fine {
        return Err(ApiError::BadRequest(format!(
            "{what} {v:?}: up to 128 letters, digits, '-', '_', '.' or ':'"
        )));
    }
    Ok(Some(v))
}

/// GET /speech/settings
#[utoipa::path(
    get,
    path = "/api/v1/speech/settings",
    tag = "speech",
    operation_id = "speech_get_settings",
    responses(
        (status = 200, description = "OK", body = SpeechSettingsResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn get_settings(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: TenantContext,
) -> Result<Json<SpeechSettingsResponse>, ApiError> {
    crate::auth::check_permission(&identity, Permission::PromptRead, state.deployment_mode)?;
    let stored = store(&state)?
        .get_tenant_speech(&tenant.0)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let configured = stored.is_some();
    let speech = stored.unwrap_or_else(|| TenantSpeech::defaults(tenant.0));
    Ok(Json(SpeechSettingsResponse::of(speech, configured)))
}

/// PUT /speech/settings
#[utoipa::path(
    put,
    path = "/api/v1/speech/settings",
    tag = "speech",
    operation_id = "speech_put_settings",
    request_body = SpeechSettingsRequest,
    responses(
        (status = 200, description = "OK", body = SpeechSettingsResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn put_settings(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: TenantContext,
    Json(req): Json<SpeechSettingsRequest>,
) -> Result<Json<SpeechSettingsResponse>, ApiError> {
    crate::auth::check_permission(&identity, Permission::PromptManage, state.deployment_mode)?;
    if req.voices.len() > 50 {
        return Err(ApiError::BadRequest("At most 50 voices by language".into()));
    }
    let mut voices = BTreeMap::new();
    for (language, voice) in req.voices {
        let language = checked("language", Some(language))?
            .ok_or_else(|| ApiError::BadRequest("A voice names its language".into()))?;
        if let Some(voice) = checked("voice", Some(voice))? {
            voices.insert(language, voice);
        }
    }
    let speech = TenantSpeech {
        tenant_id: tenant.0,
        provider: checked("provider", req.provider)?,
        voice: checked("voice", req.voice)?,
        language: checked("language", req.language)?,
        voices,
        updated_at: SystemTime::now(),
    };
    store(&state)?
        .put_tenant_speech(&speech)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    announce(&state, &tenant.0).await;
    Ok(Json(SpeechSettingsResponse::of(speech, true)))
}

/// DELETE /speech/settings — back to the platform's defaults.
#[utoipa::path(
    delete,
    path = "/api/v1/speech/settings",
    tag = "speech",
    operation_id = "speech_delete_settings",
    responses(
        (status = 204, description = "Back to the platform's defaults"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn delete_settings(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: TenantContext,
) -> Result<StatusCode, ApiError> {
    crate::auth::check_permission(&identity, Permission::PromptManage, state.deployment_mode)?;
    store(&state)?
        .delete_tenant_speech(&tenant.0)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    announce(&state, &tenant.0).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn announce(state: &ApiState, tenant: &fcp_core::TenantId) {
    let _ = state
        .event_bus
        .publish(PlatformEvent::System(SystemEvent::ConfigChanged {
            entity_type: "speech".to_string(),
            entity_id: tenant.to_string(),
            tenant_id: *tenant,
            timestamp: SystemTime::now(),
        }))
        .await;
}
