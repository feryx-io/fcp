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

fn tts(state: &ApiState) -> Result<&Arc<fcp_media::tts::TtsService>, ApiError> {
    state.tts_service.as_ref().ok_or_else(|| {
        ApiError::ServiceUnavailable(
            "No TTS service here: set [tts] remote_url on the admin".into(),
        )
    })
}

/// A provider the tenant may speak with.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SpeechProvider {
    pub name: String,
    /// The TTS server's default: what a tenant without a choice gets.
    pub is_default: bool,
}

/// GET /speech/providers — the TTS server's providers.
#[utoipa::path(
    get,
    path = "/api/v1/speech/providers",
    tag = "speech",
    operation_id = "speech_list_providers",
    responses(
        (status = 200, description = "OK", body = crate::page::Page<SpeechProvider>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_providers(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
) -> Result<Json<crate::page::Page<SpeechProvider>>, ApiError> {
    crate::auth::check_permission(&identity, Permission::PromptRead, state.deployment_mode)?;
    // No TTS service here: none to choose from.
    let Some(tts) = state.tts_service.as_ref() else {
        return Ok(Json(crate::page::Page::new(Vec::new(), None, Some(0))));
    };
    let mut list: Vec<SpeechProvider> = tts
        .providers()
        .await
        .map_err(|e| ApiError::ServiceUnavailable(format!("TTS providers: {e:#}")))?
        .into_iter()
        .map(|p| SpeechProvider {
            name: p.name,
            is_default: p.is_default,
        })
        .collect();
    list.sort_by(|a, b| b.is_default.cmp(&a.is_default).then(a.name.cmp(&b.name)));
    let total = list.len() as u64;
    Ok(Json(crate::page::Page::new(list, None, Some(total))))
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct VoicesQuery {
    /// A provider by name; its default when unset.
    pub provider: Option<String>,
    /// `en-US` unless given.
    pub language: Option<String>,
}

/// A voice a provider speaks.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SpeechVoice {
    pub name: String,
    pub display_name: String,
    pub language: String,
    pub gender: Option<String>,
}

/// GET /speech/voices — a provider's voices for a language, from the TTS
/// server: what the Speech page, Prompts and the flow builder offer.
#[utoipa::path(
    get,
    path = "/api/v1/speech/voices",
    tag = "speech",
    operation_id = "speech_list_voices",
    params(VoicesQuery),
    responses(
        (status = 200, description = "OK", body = crate::page::Page<SpeechVoice>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_voices(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    axum::extract::Query(q): axum::extract::Query<VoicesQuery>,
) -> Result<Json<crate::page::Page<SpeechVoice>>, ApiError> {
    crate::auth::check_permission(&identity, Permission::PromptRead, state.deployment_mode)?;
    let provider = checked("provider", q.provider)?;
    let language = checked("language", q.language)?.unwrap_or_else(|| "en-US".into());
    let Some(tts) = state.tts_service.as_ref() else {
        return Ok(Json(crate::page::Page::new(Vec::new(), None, Some(0))));
    };
    let voices = tts
        .voices(&language, provider.as_deref())
        .await
        .map_err(|e| ApiError::ServiceUnavailable(format!("TTS voices: {e:#}")))?;
    let list: Vec<SpeechVoice> = voices
        .into_iter()
        .map(|v| SpeechVoice {
            name: v.name,
            display_name: v.display_name,
            language: v.language,
            gender: v.gender,
        })
        .collect();
    let total = list.len() as u64;
    Ok(Json(crate::page::Page::new(list, None, Some(total))))
}

/// What to speak for a preview; unset is the tenant's choice, then the
/// TTS server's default.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct SpeechPreviewRequest {
    pub text: String,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub voice: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
}

/// POST /speech/preview — a sentence spoken now, as WAV.
#[utoipa::path(
    post,
    path = "/api/v1/speech/preview",
    tag = "speech",
    operation_id = "speech_preview",
    request_body = SpeechPreviewRequest,
    responses(
        (status = 200, description = "WAV audio", content_type = "audio/wav", body = Vec<u8>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn preview(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: TenantContext,
    Json(req): Json<SpeechPreviewRequest>,
) -> Result<impl axum::response::IntoResponse, ApiError> {
    crate::auth::check_permission(&identity, Permission::PromptRead, state.deployment_mode)?;
    let text = req.text.trim();
    if text.is_empty() || text.chars().count() > 500 {
        return Err(ApiError::BadRequest(
            "A preview speaks 1 to 500 characters".into(),
        ));
    }
    let provider = checked("provider", req.provider)?;
    let voice = checked("voice", req.voice)?;
    let language = checked("language", req.language)?;
    let audio = tts(&state)?
        .preview(
            text,
            language.as_deref(),
            voice.as_deref(),
            Some(&tenant.0),
            provider.as_deref(),
        )
        .await
        .map_err(|e| ApiError::ServiceUnavailable(format!("TTS synthesis failed: {e:#}")))?;
    Ok((
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "audio/wav")],
        audio,
    ))
}
