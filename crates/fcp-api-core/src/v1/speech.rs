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
    /// What hears the tenant (`docs/TENANT_RECOGNITION.md`): one of the
    /// server's recognizers or an account of the tenant's that hears; null
    /// is the server's default.
    pub recognizer: Option<String>,
    /// What the tenant is heard in when a request names nothing; null is
    /// `language`.
    pub recognition_language: Option<String>,
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
            recognizer: s.recognizer,
            recognition_language: s.recognition_language,
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
    #[serde(default)]
    pub recognizer: Option<String>,
    #[serde(default)]
    pub recognition_language: Option<String>,
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
    let provider = checked("provider", req.provider)?;
    known_provider(&state, &identity, &tenant.0, provider.as_deref()).await?;
    able(&state, &tenant.0, provider.as_deref(), Ability::Speaks).await?;
    let recognizer = checked("recognizer", req.recognizer)?;
    known_provider(&state, &identity, &tenant.0, recognizer.as_deref()).await?;
    able(&state, &tenant.0, recognizer.as_deref(), Ability::Hears).await?;
    let speech = TenantSpeech {
        tenant_id: tenant.0,
        provider,
        voice: checked("voice", req.voice)?,
        language: checked("language", req.language)?,
        voices,
        recognizer,
        recognition_language: checked("recognition_language", req.recognition_language)?,
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
    /// What names it in the settings and requests (`account:<id>` for one
    /// of the tenant's accounts).
    pub name: String,
    /// What to show: the provider's name, or the account's.
    pub label: String,
    /// The TTS server's default: what a tenant without a choice gets.
    pub is_default: bool,
    /// One of the tenant's own accounts (`docs/TENANT_SPEECH.md` S3).
    pub account: bool,
    /// Whether it speaks: a voice provider (`docs/TENANT_RECOGNITION.md`).
    pub speaks: bool,
    /// Whether it hears: a recognizer.
    pub hears: bool,
    /// The server's default recognizer.
    pub is_default_recognizer: bool,
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
    tenant: TenantContext,
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
            label: p.name.clone(),
            name: p.name,
            is_default: p.is_default,
            account: false,
            speaks: p.speaks,
            hears: p.hears,
            is_default_recognizer: p.is_default_recognizer,
        })
        .collect();
    list.sort_by(|a, b| b.is_default.cmp(&a.is_default).then(a.name.cmp(&b.name)));
    // A tenant sees what the platform offers it; the defaults always are.
    if !identity.is_platform_admin() {
        let held = held_back(&state).await?;
        list.retain(|p| p.is_default || p.is_default_recognizer || !held.contains(&p.name));
    }
    // Then the tenant's own accounts, by name.
    if let Ok(store) = accounts(&state) {
        let mine = store
            .list_speech_accounts(&tenant.0)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        list.extend(mine.into_iter().map(|a| SpeechProvider {
            name: format!("{ACCOUNT_PREFIX}{}", a.id),
            speaks: a.kind.speaks(),
            hears: a.kind.hears(),
            label: a.name,
            is_default: false,
            account: true,
            is_default_recognizer: false,
        }));
    }
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
    tenant: TenantContext,
    axum::extract::Query(q): axum::extract::Query<VoicesQuery>,
) -> Result<Json<crate::page::Page<SpeechVoice>>, ApiError> {
    crate::auth::check_permission(&identity, Permission::PromptRead, state.deployment_mode)?;
    let provider = checked("provider", q.provider)?;
    known_provider(&state, &identity, &tenant.0, provider.as_deref()).await?;
    let language = checked("language", q.language)?.unwrap_or_else(|| "en-US".into());
    let Some(tts) = state.tts_service.as_ref() else {
        return Ok(Json(crate::page::Page::new(Vec::new(), None, Some(0))));
    };
    let voices = tts
        .voices(&language, provider.as_deref(), Some(&tenant.0.to_string()))
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
        (status = 429, description = "This month's speech cap on the provider is reached (`speech_cap_reached`)", body = crate::error::ErrorResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn preview(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: TenantContext,
    Json(req): Json<SpeechPreviewRequest>,
) -> Result<axum::response::Response, ApiError> {
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
    known_provider(&state, &identity, &tenant.0, provider.as_deref()).await?;
    let audio = tts(&state)?
        .preview(
            text,
            language.as_deref(),
            voice.as_deref(),
            Some(&tenant.0),
            provider.as_deref(),
        )
        .await;
    spoken(audio, "TTS synthesis failed")
}

// ── A tenant's own accounts (docs/TENANT_SPEECH.md S3) ─────────────────────

use fcp_core::speech::{SpeechAccount, SpeechAccountKind, SpeechAccountStore, ACCOUNT_PREFIX};

fn accounts(state: &ApiState) -> Result<&Arc<dyn SpeechAccountStore>, ApiError> {
    state
        .storage
        .as_ref()
        .and_then(|s| s.speech_account_store())
        .ok_or_else(|| ApiError::ServiceUnavailable("Speech accounts are not kept here".into()))
}

/// A tenant's account with a TTS vendor, its key never shown.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SpeechAccountView {
    /// `speech-<uuid>`.
    pub id: String,
    pub name: String,
    pub kind: SpeechAccountKind,
    /// What names it as a provider in the settings and requests.
    pub provider: String,
    pub region: Option<String>,
    pub model: Option<String>,
    pub endpoint: Option<String>,
    pub access_key_id: Option<String>,
    /// Whether a key is kept; the key itself is never returned.
    pub has_key: bool,
    /// What the vendor charges per million characters, for an estimated
    /// cost (`docs/SPEECH_CAPS.md`).
    pub price_per_million: Option<f64>,
    pub created_at: String,
    pub updated_at: String,
}

impl SpeechAccountView {
    fn of(a: &SpeechAccount) -> Self {
        Self {
            id: format!("{ID_PREFIX}{}", a.id),
            name: a.name.clone(),
            kind: a.kind,
            provider: format!("{ACCOUNT_PREFIX}{}", a.id),
            region: a.region.clone(),
            model: a.model.clone(),
            endpoint: a.endpoint.clone(),
            access_key_id: a.access_key_id.clone(),
            has_key: a.secret.as_deref().is_some_and(|s| !s.is_empty()),
            price_per_million: a.price_per_million,
            created_at: crate::time::format_system_time(a.created_at),
            updated_at: crate::time::format_system_time(a.updated_at),
        }
    }
}

/// An account to make or change. On a change, an unset `secret` keeps the
/// one kept.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SpeechAccountRequest {
    pub name: String,
    pub kind: SpeechAccountKind,
    #[serde(default)]
    pub region: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub access_key_id: Option<String>,
    /// Write-only: the API key, subscription key or AWS secret access key.
    #[serde(default)]
    pub secret: Option<String>,
    /// What the vendor charges per million characters; null for none.
    #[serde(default)]
    pub price_per_million: Option<f64>,
}

/// A price per million characters: a number from 0 to a million.
fn price(p: Option<f64>) -> Result<Option<f64>, ApiError> {
    match p {
        Some(p) if !(0.0..=1_000_000.0).contains(&p) || !p.is_finite() => Err(
            ApiError::BadRequest("A price per million characters is 0 to 1,000,000".into()),
        ),
        p => Ok(p),
    }
}

fn account_from(
    req: SpeechAccountRequest,
    tenant: fcp_core::TenantId,
    kept: Option<&SpeechAccount>,
) -> Result<SpeechAccount, ApiError> {
    let name = req.name.trim().to_string();
    if name.is_empty() || name.chars().count() > 100 {
        return Err(ApiError::BadRequest(
            "An account's name is 1 to 100 characters".into(),
        ));
    }
    let region = checked("region", req.region)?;
    if req.kind.needs_region() && region.is_none() {
        return Err(ApiError::BadRequest(format!(
            "A {} account names its region",
            req.kind.as_str()
        )));
    }
    let endpoint = checked("endpoint", req.endpoint)?;
    if let Some(e) = &endpoint {
        let url =
            url::Url::parse(e).map_err(|_| ApiError::BadRequest("The endpoint is a URL".into()))?;
        if url.scheme() != "https" {
            return Err(ApiError::BadRequest("The endpoint is https".into()));
        }
    }
    let access_key_id = checked("access_key_id", req.access_key_id)?;
    if req.kind == SpeechAccountKind::AwsPolly && access_key_id.is_none() {
        return Err(ApiError::BadRequest(
            "An AWS account names its access key id".into(),
        ));
    }
    let secret = match req
        .secret
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        Some(s) if s.len() > 4096 => {
            return Err(ApiError::BadRequest("A key is at most 4096 bytes".into()))
        }
        Some(s) => Some(s),
        None => kept.and_then(|k| k.secret.clone()),
    };
    if secret.is_none() {
        return Err(ApiError::BadRequest("An account has a key".into()));
    }
    let now = SystemTime::now();
    Ok(SpeechAccount {
        id: kept.map(|k| k.id).unwrap_or_else(uuid::Uuid::new_v4),
        tenant_id: tenant,
        name,
        kind: req.kind,
        region,
        model: checked("model", req.model)?,
        endpoint,
        access_key_id,
        secret,
        price_per_million: price(req.price_per_million)?,
        created_at: kept.map(|k| k.created_at).unwrap_or(now),
        updated_at: now,
    })
}

async fn name_taken(
    state: &ApiState,
    tenant: &fcp_core::TenantId,
    name: &str,
    other_than: Option<uuid::Uuid>,
) -> Result<bool, ApiError> {
    Ok(accounts(state)?
        .list_speech_accounts(tenant)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .iter()
        .any(|a| a.name.eq_ignore_ascii_case(name) && Some(a.id) != other_than))
}

/// An account's id as the API shows it: `speech-<uuid>`.
const ID_PREFIX: &str = "speech-";

fn account_id(id: &str) -> Result<uuid::Uuid, ApiError> {
    id.strip_prefix(ID_PREFIX)
        .unwrap_or(id)
        .parse()
        .map_err(|_| ApiError::NotFound("No such speech account".into()))
}

/// GET /speech/accounts
#[utoipa::path(
    get,
    path = "/api/v1/speech/accounts",
    tag = "speech",
    operation_id = "speech_list_accounts",
    responses(
        (status = 200, description = "OK", body = crate::page::Page<SpeechAccountView>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_accounts(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: TenantContext,
) -> Result<Json<crate::page::Page<SpeechAccountView>>, ApiError> {
    crate::auth::check_permission(&identity, Permission::PromptRead, state.deployment_mode)?;
    let list: Vec<SpeechAccountView> = accounts(&state)?
        .list_speech_accounts(&tenant.0)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .iter()
        .map(SpeechAccountView::of)
        .collect();
    let total = list.len() as u64;
    Ok(Json(crate::page::Page::new(list, None, Some(total))))
}

/// POST /speech/accounts
#[utoipa::path(
    post,
    path = "/api/v1/speech/accounts",
    tag = "speech",
    operation_id = "speech_create_account",
    request_body = SpeechAccountRequest,
    responses(
        (status = 201, description = "Made", body = SpeechAccountView),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn create_account(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: TenantContext,
    Json(req): Json<SpeechAccountRequest>,
) -> Result<(StatusCode, Json<SpeechAccountView>), ApiError> {
    crate::auth::check_permission(&identity, Permission::PromptManage, state.deployment_mode)?;
    let store = accounts(&state)?;
    if store
        .list_speech_accounts(&tenant.0)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .len()
        >= 20
    {
        return Err(ApiError::BadRequest("At most 20 speech accounts".into()));
    }
    let account = account_from(req, tenant.0, None)?;
    if name_taken(&state, &tenant.0, &account.name, None).await? {
        return Err(ApiError::Conflict("An account has that name".into()));
    }
    store
        .put_speech_account(&account)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok((StatusCode::CREATED, Json(SpeechAccountView::of(&account))))
}

/// PUT /speech/accounts/{id}
#[utoipa::path(
    put,
    path = "/api/v1/speech/accounts/{id}",
    tag = "speech",
    operation_id = "speech_update_account",
    params(("id" = String, Path, description = "The account")),
    request_body = SpeechAccountRequest,
    responses(
        (status = 200, description = "OK", body = SpeechAccountView),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn update_account(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: TenantContext,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(req): Json<SpeechAccountRequest>,
) -> Result<Json<SpeechAccountView>, ApiError> {
    crate::auth::check_permission(&identity, Permission::PromptManage, state.deployment_mode)?;
    let id = account_id(&id)?;
    let store = accounts(&state)?;
    let kept = store
        .get_speech_account(&tenant.0, &id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::NotFound("No such speech account".into()))?;
    if req.kind != kept.kind && req.secret.as_deref().is_none_or(|s| s.trim().is_empty()) {
        return Err(ApiError::BadRequest(
            "Another vendor needs its own key".into(),
        ));
    }
    let account = account_from(req, tenant.0, Some(&kept))?;
    if name_taken(&state, &tenant.0, &account.name, Some(id)).await? {
        return Err(ApiError::Conflict("An account has that name".into()));
    }
    store
        .put_speech_account(&account)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    announce(&state, &tenant.0).await;
    Ok(Json(SpeechAccountView::of(&account)))
}

/// DELETE /speech/accounts/{id} — and the settings that chose it go back
/// to the platform's provider.
#[utoipa::path(
    delete,
    path = "/api/v1/speech/accounts/{id}",
    tag = "speech",
    operation_id = "speech_delete_account",
    params(("id" = String, Path, description = "The account")),
    responses(
        (status = 204, description = "Deleted"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn delete_account(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: TenantContext,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<StatusCode, ApiError> {
    crate::auth::check_permission(&identity, Permission::PromptManage, state.deployment_mode)?;
    let id = account_id(&id)?;
    if !accounts(&state)?
        .delete_speech_account(&tenant.0, &id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
    {
        return Err(ApiError::NotFound("No such speech account".into()));
    }
    // Its voices belong to it: a choice of it is a choice of nothing now.
    let settings = store(&state)?;
    if let Some(mut s) = settings
        .get_tenant_speech(&tenant.0)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
    {
        if s.provider.as_deref().and_then(fcp_core::speech::account_of) == Some(id) {
            s.provider = None;
            s.voice = None;
            s.voices.clear();
            s.updated_at = SystemTime::now();
            settings
                .put_tenant_speech(&s)
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
        }
    }
    announce(&state, &tenant.0).await;
    Ok(StatusCode::NO_CONTENT)
}

/// A provider named in the settings or a request: one of the TTS
/// server's, or an account of this tenant's.
async fn known_provider(
    state: &ApiState,
    identity: &AuthIdentity,
    tenant: &fcp_core::TenantId,
    provider: Option<&str>,
) -> Result<(), ApiError> {
    let Some(p) = provider else { return Ok(()) };
    if !p.starts_with(ACCOUNT_PREFIX) {
        // One of the platform's: offered to tenants unless held back.
        if !identity.is_platform_admin() && held_back(state).await?.iter().any(|h| h == p) {
            return Err(ApiError::Forbidden(format!(
                "The platform does not offer {p} to tenants"
            )));
        }
        return Ok(());
    }
    let found = match fcp_core::speech::account_of(p) {
        Some(id) => accounts(state)?
            .get_speech_account(tenant, &id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
            .is_some(),
        None => false,
    };
    if found {
        Ok(())
    } else {
        Err(ApiError::BadRequest(format!("No such speech account: {p}")))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Ability {
    Speaks,
    Hears,
}

/// A provider chosen to speak, or a recognizer chosen to hear, can: an
/// account by its kind, one of the server's by what the server says (when
/// it answers) (`docs/TENANT_RECOGNITION.md` R2).
async fn able(
    state: &ApiState,
    tenant: &fcp_core::TenantId,
    provider: Option<&str>,
    ability: Ability,
) -> Result<(), ApiError> {
    let Some(p) = provider else { return Ok(()) };
    let what = match ability {
        Ability::Speaks => "speak",
        Ability::Hears => "hear",
    };
    let can = match fcp_core::speech::account_of(p) {
        Some(id) => match accounts(state)?
            .get_speech_account(tenant, &id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
        {
            Some(a) => match ability {
                Ability::Speaks => a.kind.speaks(),
                Ability::Hears => a.kind.hears(),
            },
            None => true,
        },
        None => match state.tts_service.as_ref() {
            Some(tts) => match tts.providers().await {
                Ok(list) => match list.iter().find(|x| x.name == p) {
                    Some(x) => match ability {
                        Ability::Speaks => x.speaks,
                        Ability::Hears => x.hears,
                    },
                    None => {
                        return Err(ApiError::BadRequest(format!(
                            "The speech server has no provider {p}"
                        )))
                    }
                },
                Err(_) => true,
            },
            None => true,
        },
    };
    if can {
        Ok(())
    } else {
        Err(ApiError::BadRequest(format!("{p} does not {what}")))
    }
}

/// The platform's providers held back from tenants.
async fn held_back(state: &ApiState) -> Result<Vec<String>, ApiError> {
    match state.storage.as_ref().and_then(|s| s.speech_store()) {
        Some(store) => store
            .held_back_providers()
            .await
            .map_err(|e| ApiError::Internal(e.to_string())),
        None => Ok(Vec::new()),
    }
}

/// POST /speech/accounts/{id}/test — a sentence spoken through the
/// account, as WAV: whether its key works.
#[utoipa::path(
    post,
    path = "/api/v1/speech/accounts/{id}/test",
    tag = "speech",
    operation_id = "speech_test_account",
    params(("id" = String, Path, description = "The account")),
    responses(
        (status = 200, description = "The account spoke", content_type = "audio/wav", body = Vec<u8>),
        (status = 429, description = "This month's speech cap on the provider is reached (`speech_cap_reached`)", body = crate::error::ErrorResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn test_account(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: TenantContext,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<axum::response::Response, ApiError> {
    crate::auth::check_permission(&identity, Permission::PromptManage, state.deployment_mode)?;
    let id = account_id(&id)?;
    let account = accounts(&state)?
        .get_speech_account(&tenant.0, &id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::NotFound("No such speech account".into()))?;
    let provider = format!("{ACCOUNT_PREFIX}{}", account.id);
    let audio = tts(&state)?
        .preview(
            &format!("This is {}.", account.name),
            None,
            None,
            Some(&tenant.0),
            Some(&provider),
        )
        .await;
    spoken(audio, "The account could not speak")
}

// ── The platform's offer (docs/TENANT_SPEECH.md S3) ────────────────────────

/// One of the TTS server's providers, as the platform offers it.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct OfferedProvider {
    pub name: String,
    pub is_default: bool,
    /// Whether it speaks, and whether it hears (`docs/TENANT_RECOGNITION.md`).
    pub speaks: bool,
    pub hears: bool,
    pub is_default_recognizer: bool,
    /// Whether tenants may choose it.
    pub offered: bool,
    /// What the platform charges tenants per million characters, for an
    /// estimated cost beside their usage (`docs/SPEECH_CAPS.md`).
    pub price_per_million: Option<f64>,
}

/// Which of the TTS server's providers tenants may choose.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct PlatformSpeech {
    pub providers: Vec<OfferedProvider>,
    /// Held back from tenants (a provider the server no longer has may
    /// still be named here).
    pub held_back: Vec<String>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PlatformSpeechRequest {
    pub held_back: Vec<String>,
    /// Prices per million characters by provider; absent keeps them, and a
    /// provider left out has none.
    #[serde(default)]
    pub prices: Option<BTreeMap<String, f64>>,
}

fn require_platform(identity: &AuthIdentity) -> Result<(), ApiError> {
    if identity.is_platform_admin() {
        Ok(())
    } else {
        Err(ApiError::Forbidden("Platform admin access required".into()))
    }
}

/// The platform's prices per million characters by provider.
async fn prices(state: &ApiState) -> Result<BTreeMap<String, f64>, ApiError> {
    match state.storage.as_ref().and_then(|s| s.speech_store()) {
        Some(store) => store
            .speech_prices()
            .await
            .map_err(|e| ApiError::Internal(e.to_string())),
        None => Ok(BTreeMap::new()),
    }
}

async fn platform_speech(state: &ApiState) -> Result<PlatformSpeech, ApiError> {
    let held_back = held_back(state).await?;
    let prices = prices(state).await?;
    let providers = match state.tts_service.as_ref() {
        Some(tts) => tts
            .providers()
            .await
            .map_err(|e| ApiError::ServiceUnavailable(format!("TTS providers: {e:#}")))?,
        None => Vec::new(),
    };
    Ok(PlatformSpeech {
        providers: providers
            .into_iter()
            .map(|p| OfferedProvider {
                offered: p.is_default || p.is_default_recognizer || !held_back.contains(&p.name),
                speaks: p.speaks,
                hears: p.hears,
                is_default_recognizer: p.is_default_recognizer,
                price_per_million: prices.get(&p.name).copied(),
                name: p.name,
                is_default: p.is_default,
            })
            .collect(),
        held_back,
    })
}

/// GET /system/speech
#[utoipa::path(
    get,
    path = "/api/v1/system/speech",
    tag = "speech",
    operation_id = "system_get_speech",
    responses(
        (status = 200, description = "OK", body = PlatformSpeech),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn get_platform(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
) -> Result<Json<PlatformSpeech>, ApiError> {
    require_platform(&identity)?;
    Ok(Json(platform_speech(&state).await?))
}

/// PUT /system/speech — the providers held back from tenants.
#[utoipa::path(
    put,
    path = "/api/v1/system/speech",
    tag = "speech",
    operation_id = "system_put_speech",
    request_body = PlatformSpeechRequest,
    responses(
        (status = 200, description = "OK", body = PlatformSpeech),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn put_platform(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    Json(req): Json<PlatformSpeechRequest>,
) -> Result<Json<PlatformSpeech>, ApiError> {
    require_platform(&identity)?;
    if req.held_back.len() > 50 {
        return Err(ApiError::BadRequest(
            "At most 50 providers held back".into(),
        ));
    }
    let mut held: Vec<String> = Vec::new();
    for p in req.held_back {
        let p = checked("provider", Some(p))?
            .ok_or_else(|| ApiError::BadRequest("A provider has a name".into()))?;
        if p.starts_with(ACCOUNT_PREFIX) {
            return Err(ApiError::BadRequest(
                "A tenant's own account is not the platform's".into(),
            ));
        }
        if !held.contains(&p) {
            held.push(p);
        }
    }
    if let Some(tts) = state.tts_service.as_ref() {
        if let Ok(providers) = tts.providers().await {
            if providers
                .iter()
                .any(|p| (p.is_default || p.is_default_recognizer) && held.contains(&p.name))
            {
                return Err(ApiError::BadRequest(
                    "The default provider and recognizer serve tenants that choose none: neither can be held back"
                        .into(),
                ));
            }
        }
    }
    if let Some(prices) = &req.prices {
        if prices.len() > 50 {
            return Err(ApiError::BadRequest("At most 50 prices".into()));
        }
        for (p, v) in prices {
            if p.starts_with(ACCOUNT_PREFIX) {
                return Err(ApiError::BadRequest(
                    "A tenant's own account's price is the tenant's".into(),
                ));
            }
            price(Some(*v))?;
        }
    }
    store(&state)?
        .put_held_back_providers(&held)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if let Some(prices) = &req.prices {
        store(&state)?
            .put_speech_prices(prices)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
    }
    Ok(Json(platform_speech(&state).await?))
}

// ── Usage (docs/TENANT_SPEECH.md S4) ───────────────────────────────────────

#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct UsageQuery {
    /// RFC 3339 or `YYYY-MM-DD`; twelve months back, from the month's
    /// start, unless given.
    pub from: Option<String>,
    /// RFC 3339 or `YYYY-MM-DD`, exclusive; now unless given.
    pub to: Option<String>,
    /// `month` (the default) or `day`, in UTC.
    pub by: Option<String>,
}

/// What the tenant's speech cost in one period with one provider.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SpeechUsagePeriod {
    /// `YYYY-MM` or `YYYY-MM-DD` (UTC).
    pub period: String,
    /// A platform provider's name, or `account:<uuid>`.
    pub provider: String,
    /// What to show: the provider, or the account's name.
    pub label: String,
    pub requests: u64,
    pub characters: u64,
    /// What the provider spoke, and bills for; the rest came from the cache.
    pub synthesized_characters: u64,
    /// The synthesized characters at the provider's price per million
    /// (the platform's, or the account's own), where one is set.
    pub estimated_cost: Option<f64>,
}

fn usage_time(what: &str, s: &str) -> Result<chrono::DateTime<chrono::Utc>, ApiError> {
    use chrono::{NaiveDate, TimeZone, Utc};
    if let Ok(t) = chrono::DateTime::parse_from_rfc3339(s) {
        return Ok(t.with_timezone(&Utc));
    }
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .ok()
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map(|d| Utc.from_utc_datetime(&d))
        .ok_or_else(|| ApiError::BadRequest(format!("{what} is RFC 3339 or YYYY-MM-DD")))
}

/// GET /speech/usage — the tenant's speech by month or day and provider.
#[utoipa::path(
    get,
    path = "/api/v1/speech/usage",
    tag = "speech",
    operation_id = "speech_usage",
    params(UsageQuery),
    responses(
        (status = 200, description = "OK", body = crate::page::Page<SpeechUsagePeriod>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn usage(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: TenantContext,
    axum::extract::Query(q): axum::extract::Query<UsageQuery>,
) -> Result<Json<crate::page::Page<SpeechUsagePeriod>>, ApiError> {
    use chrono::{Datelike, TimeZone, Utc};
    crate::auth::check_permission(&identity, Permission::PromptRead, state.deployment_mode)?;
    let store = state
        .storage
        .as_ref()
        .and_then(|s| s.speech_usage_store())
        .ok_or_else(|| ApiError::ServiceUnavailable("Speech usage is not kept here".into()))?;
    let by_day = match q.by.as_deref() {
        None | Some("month") => false,
        Some("day") => true,
        Some(_) => return Err(ApiError::BadRequest("by is month or day".into())),
    };
    let now = Utc::now();
    let to = match q.to.as_deref() {
        Some(s) => usage_time("to", s)?,
        None => now,
    };
    let from = match q.from.as_deref() {
        Some(s) => usage_time("from", s)?,
        None => {
            let (mut y, mut m) = (now.year(), now.month());
            for _ in 0..11 {
                if m == 1 {
                    y -= 1;
                    m = 12;
                } else {
                    m -= 1;
                }
            }
            Utc.with_ymd_and_hms(y, m, 1, 0, 0, 0)
                .single()
                .unwrap_or(now)
        }
    };
    if from >= to {
        return Err(ApiError::BadRequest("from is before to".into()));
    }
    if to - from > chrono::Duration::days(800) {
        return Err(ApiError::BadRequest("At most 800 days at once".into()));
    }
    let hours = store
        .speech_usage(&tenant.0, from.into(), to.into())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    // Accounts by their names, for the label; a deleted one keeps its id.
    // Prices: the platform's, and each account's own.
    let mut unit_prices = prices(&state).await?;
    let names: std::collections::HashMap<String, String> = match accounts(&state) {
        Ok(store) => store
            .list_speech_accounts(&tenant.0)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
            .into_iter()
            .map(|a| {
                let provider = format!("{ACCOUNT_PREFIX}{}", a.id);
                if let Some(p) = a.price_per_million {
                    unit_prices.insert(provider.clone(), p);
                }
                (provider, a.name)
            })
            .collect(),
        Err(_) => Default::default(),
    };
    unit_prices.retain(|p, _| !p.starts_with(ACCOUNT_PREFIX) || names.contains_key(p));
    let mut periods: BTreeMap<(String, String), SpeechUsagePeriod> = BTreeMap::new();
    for h in hours {
        let at: chrono::DateTime<Utc> = h.hour.into();
        let period = if by_day {
            at.format("%Y-%m-%d").to_string()
        } else {
            at.format("%Y-%m").to_string()
        };
        let p = periods
            .entry((period.clone(), h.provider.clone()))
            .or_insert_with(|| SpeechUsagePeriod {
                label: names.get(&h.provider).cloned().unwrap_or_else(|| {
                    if h.provider.starts_with(ACCOUNT_PREFIX) {
                        "A deleted account".to_string()
                    } else {
                        h.provider.clone()
                    }
                }),
                period,
                provider: h.provider.clone(),
                requests: 0,
                characters: 0,
                synthesized_characters: 0,
                estimated_cost: None,
            });
        p.requests += h.requests;
        p.characters += h.characters;
        p.synthesized_characters += h.synthesized_characters;
    }
    let mut list: Vec<SpeechUsagePeriod> = periods.into_values().collect();
    for p in &mut list {
        p.estimated_cost = unit_prices.get(&p.provider).map(|price| {
            (p.synthesized_characters as f64 / 1_000_000.0 * price * 10_000.0).round() / 10_000.0
        });
    }
    let total = list.len() as u64;
    Ok(Json(crate::page::Page::new(list, None, Some(total))))
}

// ── Caps (docs/SPEECH_CAPS.md C1) ──────────────────────────────────────────

use fcp_core::speech::{effective_cap, month_of, SpeechCap, SpeechCapKind, SpeechUsageStore};

/// The most a cap may be: a trillion characters a month.
const MAX_CAP: u64 = 1_000_000_000_000;

fn usage_store(state: &ApiState) -> Result<&Arc<dyn SpeechUsageStore>, ApiError> {
    state
        .storage
        .as_ref()
        .and_then(|s| s.speech_usage_store())
        .ok_or_else(|| ApiError::ServiceUnavailable("Speech caps are not kept here".into()))
}

/// One provider's cap on the tenant this month.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SpeechCapView {
    /// A platform provider's name, or `account:<uuid>`.
    pub provider: String,
    /// The provider, or the account's name.
    pub label: String,
    /// What the platform holds the tenant to on this provider (its cap
    /// for this tenant, else its default); never on an account.
    pub platform_cap: Option<u64>,
    /// Whether the platform set a cap for this tenant itself, rather than
    /// its default (null `platform_cap` then means none for this tenant).
    pub platform_override: bool,
    /// The tenant's own cap.
    pub own_cap: Option<u64>,
    /// The cap that applies: the lowest of the two.
    pub cap: Option<u64>,
    /// Synthesized characters this month (UTC), the cloud's and sites'.
    pub used: u64,
    /// `ok`, `near` (80 % or more) or `reached`.
    pub state: String,
    /// When the month's count starts again (RFC 3339).
    pub resets_at: String,
}

/// A cap to keep. `monthly_characters` null is allowed only on a tenant's
/// platform cap: none for this tenant, whatever the default says.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SpeechCapRequest {
    pub monthly_characters: Option<u64>,
    /// `tenant` (the default: the tenant's own) or `platform` (a platform
    /// administrator's cap on this tenant).
    #[serde(default)]
    pub kind: Option<String>,
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct CapKindQuery {
    /// `tenant` (the default) or `platform`.
    pub kind: Option<String>,
}

fn cap_kind(kind: Option<&str>, identity: &AuthIdentity) -> Result<SpeechCapKind, ApiError> {
    match kind {
        None | Some("tenant") => Ok(SpeechCapKind::Tenant),
        Some("platform") if identity.is_platform_admin() => Ok(SpeechCapKind::Platform),
        Some("platform") => Err(ApiError::Forbidden(
            "Only a platform administrator sets the platform's caps".into(),
        )),
        Some(_) => Err(ApiError::BadRequest("kind is tenant or platform".into())),
    }
}

/// The TTS server's default provider, when the server says.
async fn default_provider(state: &ApiState) -> Option<String> {
    let tts = state.tts_service.as_ref()?;
    tts.providers()
        .await
        .ok()?
        .into_iter()
        .find(|p| p.is_default)
        .map(|p| p.name)
}

/// A provider a cap may be put on: never the default, which speaks when a
/// cap is reached; a platform provider the server has, or (for a tenant's
/// own cap) one of its accounts.
async fn cappable(
    state: &ApiState,
    identity: &AuthIdentity,
    tenant: Option<&fcp_core::TenantId>,
    provider: &str,
) -> Result<(), ApiError> {
    if provider.is_empty() || provider.len() > 200 {
        return Err(ApiError::BadRequest("A provider has a name".into()));
    }
    if provider.starts_with(ACCOUNT_PREFIX) {
        return match tenant {
            Some(t) => known_provider(state, identity, t, Some(provider)).await,
            None => Err(ApiError::BadRequest(
                "A tenant's own account is not the platform's to cap".into(),
            )),
        };
    }
    if let Some(tts) = state.tts_service.as_ref() {
        if let Ok(providers) = tts.providers().await {
            match providers.iter().find(|p| p.name == provider) {
                None => {
                    return Err(ApiError::BadRequest(format!(
                        "The TTS server has no provider {provider}"
                    )))
                }
                Some(p) if p.is_default => {
                    return Err(ApiError::BadRequest(
                        "The default provider speaks when a cap is reached: it cannot be capped"
                            .into(),
                    ))
                }
                Some(_) => {}
            }
        }
    }
    Ok(())
}

fn limit(n: Option<u64>, may_be_none: bool) -> Result<Option<u64>, ApiError> {
    match n {
        Some(n) if n > MAX_CAP => Err(ApiError::BadRequest(format!(
            "A cap is at most {MAX_CAP} characters a month"
        ))),
        None if !may_be_none => Err(ApiError::BadRequest("monthly_characters is the cap".into())),
        n => Ok(n),
    }
}

fn rfc3339(t: std::time::SystemTime) -> String {
    chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// GET /speech/caps — the caps on the tenant, and where it stands.
#[utoipa::path(
    get,
    path = "/api/v1/speech/caps",
    tag = "speech",
    operation_id = "speech_list_caps",
    responses(
        (status = 200, description = "OK", body = crate::page::Page<SpeechCapView>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_caps(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: TenantContext,
) -> Result<Json<crate::page::Page<SpeechCapView>>, ApiError> {
    crate::auth::check_permission(&identity, Permission::PromptRead, state.deployment_mode)?;
    let store = usage_store(&state)?;
    let err = |e: anyhow::Error| ApiError::Internal(e.to_string());
    let mut caps = store.speech_caps(None).await.map_err(err)?;
    caps.extend(store.speech_caps(Some(&tenant.0)).await.map_err(err)?);
    let default = default_provider(&state).await;
    let mut providers: Vec<String> = Vec::new();
    for c in &caps {
        if !providers.contains(&c.provider) && Some(&c.provider) != default.as_ref() {
            providers.push(c.provider.clone());
        }
    }
    let now = std::time::SystemTime::now();
    let (month, next) = month_of(now);
    // The month's synthesized characters; at a site, with what the cloud
    // says was spoken elsewhere.
    let used: std::collections::HashMap<String, u64> = store
        .synthesized_since(month)
        .await
        .map_err(err)?
        .into_iter()
        .filter(|(t, _, _)| *t == tenant.0)
        .map(|(_, p, n)| (p, n))
        .collect();
    let names: std::collections::HashMap<String, String> = match accounts(&state) {
        Ok(a) => a
            .list_speech_accounts(&tenant.0)
            .await
            .map_err(err)?
            .into_iter()
            .map(|a| (format!("{ACCOUNT_PREFIX}{}", a.id), a.name))
            .collect(),
        Err(_) => Default::default(),
    };
    let find = |t: Option<&fcp_core::TenantId>, p: &str, kind| {
        caps.iter()
            .find(|c| c.tenant_id.as_ref() == t && c.provider == p && c.kind == kind)
    };
    let mut list: Vec<SpeechCapView> = providers
        .into_iter()
        .map(|p| {
            let account = p.starts_with(ACCOUNT_PREFIX);
            let override_ = find(Some(&tenant.0), &p, SpeechCapKind::Platform);
            let platform_cap = if account {
                None
            } else {
                match override_ {
                    Some(c) => c.monthly_characters,
                    None => {
                        find(None, &p, SpeechCapKind::Platform).and_then(|c| c.monthly_characters)
                    }
                }
            };
            let cap = effective_cap(&caps, &tenant.0, &p);
            let n = used.get(&p).copied().unwrap_or(0);
            let state = match cap {
                Some(c) if n >= c => "reached",
                Some(c) if n.saturating_mul(5) >= c.saturating_mul(4) => "near",
                _ => "ok",
            };
            SpeechCapView {
                label: names.get(&p).cloned().unwrap_or_else(|| p.clone()),
                platform_override: !account && override_.is_some(),
                own_cap: find(Some(&tenant.0), &p, SpeechCapKind::Tenant)
                    .and_then(|c| c.monthly_characters),
                platform_cap,
                cap,
                used: n,
                state: state.into(),
                resets_at: rfc3339(next),
                provider: p,
            }
        })
        .collect();
    list.sort_by(|a, b| a.label.cmp(&b.label));
    let total = list.len() as u64;
    Ok(Json(crate::page::Page::new(list, None, Some(total))))
}

/// PUT /speech/caps/{provider} — the tenant's own cap, or (a platform
/// administrator, `kind: platform`) the platform's cap on this tenant.
#[utoipa::path(
    put,
    path = "/api/v1/speech/caps/{provider}",
    tag = "speech",
    operation_id = "speech_put_cap",
    params(("provider" = String, Path, description = "A platform provider, or account:<uuid>")),
    request_body = SpeechCapRequest,
    responses(
        (status = 204, description = "Kept"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn put_cap(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: TenantContext,
    axum::extract::Path(provider): axum::extract::Path<String>,
    Json(req): Json<SpeechCapRequest>,
) -> Result<StatusCode, ApiError> {
    crate::auth::check_permission(&identity, Permission::PromptManage, state.deployment_mode)?;
    let kind = cap_kind(req.kind.as_deref(), &identity)?;
    if kind == SpeechCapKind::Platform && provider.starts_with(ACCOUNT_PREFIX) {
        return Err(ApiError::BadRequest(
            "A tenant's own account is not the platform's to cap".into(),
        ));
    }
    cappable(&state, &identity, Some(&tenant.0), &provider).await?;
    let monthly_characters = limit(req.monthly_characters, kind == SpeechCapKind::Platform)?;
    usage_store(&state)?
        .put_speech_cap(&SpeechCap {
            tenant_id: Some(tenant.0),
            provider,
            kind,
            monthly_characters,
        })
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

/// DELETE /speech/caps/{provider}
#[utoipa::path(
    delete,
    path = "/api/v1/speech/caps/{provider}",
    tag = "speech",
    operation_id = "speech_delete_cap",
    params(
        ("provider" = String, Path, description = "A platform provider, or account:<uuid>"),
        CapKindQuery,
    ),
    responses(
        (status = 204, description = "Removed"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn delete_cap(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: TenantContext,
    axum::extract::Path(provider): axum::extract::Path<String>,
    axum::extract::Query(q): axum::extract::Query<CapKindQuery>,
) -> Result<StatusCode, ApiError> {
    crate::auth::check_permission(&identity, Permission::PromptManage, state.deployment_mode)?;
    let kind = cap_kind(q.kind.as_deref(), &identity)?;
    let gone = usage_store(&state)?
        .delete_speech_cap(Some(&tenant.0), &provider, kind)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if gone {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound("No such cap".into()))
    }
}

/// A platform provider's default cap for every tenant.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct PlatformSpeechCap {
    pub provider: String,
    pub monthly_characters: u64,
}

/// GET /system/speech/caps — the platform's default caps.
#[utoipa::path(
    get,
    path = "/api/v1/system/speech/caps",
    tag = "speech",
    operation_id = "system_list_speech_caps",
    responses(
        (status = 200, description = "OK", body = crate::page::Page<PlatformSpeechCap>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_platform_caps(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
) -> Result<Json<crate::page::Page<PlatformSpeechCap>>, ApiError> {
    require_platform(&identity)?;
    let list: Vec<PlatformSpeechCap> = usage_store(&state)?
        .speech_caps(None)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .into_iter()
        .filter(|c| c.kind == SpeechCapKind::Platform)
        .filter_map(|c| {
            Some(PlatformSpeechCap {
                monthly_characters: c.monthly_characters?,
                provider: c.provider,
            })
        })
        .collect();
    let total = list.len() as u64;
    Ok(Json(crate::page::Page::new(list, None, Some(total))))
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PlatformSpeechCapRequest {
    pub monthly_characters: u64,
}

/// PUT /system/speech/caps/{provider} — a platform provider's default cap.
#[utoipa::path(
    put,
    path = "/api/v1/system/speech/caps/{provider}",
    tag = "speech",
    operation_id = "system_put_speech_cap",
    params(("provider" = String, Path, description = "A platform provider")),
    request_body = PlatformSpeechCapRequest,
    responses(
        (status = 204, description = "Kept"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn put_platform_cap(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    axum::extract::Path(provider): axum::extract::Path<String>,
    Json(req): Json<PlatformSpeechCapRequest>,
) -> Result<StatusCode, ApiError> {
    require_platform(&identity)?;
    cappable(&state, &identity, None, &provider).await?;
    let monthly_characters = limit(Some(req.monthly_characters), false)?;
    usage_store(&state)?
        .put_speech_cap(&SpeechCap {
            tenant_id: None,
            provider,
            kind: SpeechCapKind::Platform,
            monthly_characters,
        })
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

/// DELETE /system/speech/caps/{provider}
#[utoipa::path(
    delete,
    path = "/api/v1/system/speech/caps/{provider}",
    tag = "speech",
    operation_id = "system_delete_speech_cap",
    params(("provider" = String, Path, description = "A platform provider")),
    responses(
        (status = 204, description = "Removed"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn delete_platform_cap(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    axum::extract::Path(provider): axum::extract::Path<String>,
) -> Result<StatusCode, ApiError> {
    require_platform(&identity)?;
    let gone = usage_store(&state)?
        .delete_speech_cap(None, &provider, SpeechCapKind::Platform)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if gone {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound("No such cap".into()))
    }
}

/// Audio the TTS server spoke, or its refusal at a cap answered as 429
/// with `speech_cap_reached` (`docs/SPEECH_CAPS.md`).
fn spoken(
    result: anyhow::Result<Vec<u8>>,
    failed: &str,
) -> Result<axum::response::Response, ApiError> {
    use axum::response::IntoResponse;
    match result {
        Ok(audio) => Ok((
            StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, "audio/wav")],
            audio,
        )
            .into_response()),
        Err(e) => {
            let text = format!("{e:#}");
            if !text.contains("speech_cap_reached") {
                return Err(ApiError::ServiceUnavailable(format!("{failed}: {text}")));
            }
            // The server's own words, where its JSON can be found.
            let details = text
                .find('{')
                .and_then(|i| serde_json::from_str::<serde_json::Value>(&text[i..]).ok())
                .and_then(|v| v["error"].as_str().map(str::to_string))
                .unwrap_or_else(|| "This month's speech cap on this provider is reached".into());
            Ok((
                StatusCode::TOO_MANY_REQUESTS,
                Json(crate::error::ErrorResponse {
                    error: "Too Many Requests".into(),
                    code: "speech_cap_reached".into(),
                    details: Some(details),
                }),
            )
                .into_response())
        }
    }
}

// ── Warnings (docs/SPEECH_CAPS.md C2) ──────────────────────────────────────

/// How often the admin compares the month's speech with the caps.
pub const CAP_SWEEP_EVERY: std::time::Duration = std::time::Duration::from_secs(60);

/// Raise `speech.cap_near` and `speech.cap_reached` for what passed 80 % or
/// 100 % of a cap this month and was not yet said; what was raised.
pub async fn sweep_caps(
    state: &ApiState,
    now: std::time::SystemTime,
) -> Vec<fcp_events::SpeechEvent> {
    use fcp_core::speech::CapLevel;
    // A site's caps and warnings are the cloud's (docs/SPEECH_CAPS.md §5).
    if state.managed_by.is_some() {
        return Vec::new();
    }
    let Ok(store) = usage_store(state) else {
        return Vec::new();
    };
    let caps = match store.all_speech_caps().await {
        Ok(caps) => caps,
        Err(e) => {
            tracing::warn!(error = %e, "Speech cap sweep: caps could not be read");
            return Vec::new();
        }
    };
    let (month, _) = month_of(now);
    let default = default_provider(state).await;
    // Sites are told even when nothing is capped, so a removed cap goes.
    carry_to_sites(state, &caps, month, default.as_deref()).await;
    if caps.is_empty() {
        return Vec::new();
    }
    let used = match store.synthesized_since(month).await {
        Ok(used) => used,
        Err(e) => {
            tracing::warn!(error = %e, "Speech cap sweep: usage could not be read");
            return Vec::new();
        }
    };
    let month_name = chrono::DateTime::<chrono::Utc>::from(month)
        .format("%Y-%m")
        .to_string();
    let mut raised = Vec::new();
    for (tenant, provider, n) in used {
        if Some(&provider) == default.as_ref() {
            continue;
        }
        let Some(cap) = effective_cap(&caps, &tenant, &provider) else {
            continue;
        };
        let level = CapLevel::of(n, cap);
        if level == CapLevel::Ok {
            continue;
        }
        // Reached straight from below 80 %: near is noted too, so it is
        // never said after the cap.
        let mut fresh = false;
        for l in [CapLevel::Near, CapLevel::Reached] {
            if l > level {
                break;
            }
            match store
                .note_speech_cap(&tenant, &provider, month, l, cap, n)
                .await
            {
                Ok(new) => fresh |= new && l == level,
                Err(e) => {
                    tracing::warn!(error = %e, "Speech cap sweep: a notice could not be kept");
                }
            }
        }
        if !fresh {
            continue;
        }
        let event = match level {
            CapLevel::Reached => fcp_events::SpeechEvent::CapReached {
                tenant_id: tenant,
                provider,
                cap,
                used: n,
                month: month_name.clone(),
                timestamp: now,
            },
            _ => fcp_events::SpeechEvent::CapNear {
                tenant_id: tenant,
                provider,
                cap,
                used: n,
                month: month_name.clone(),
                timestamp: now,
            },
        };
        tracing::info!(tenant_id = %event.tenant_id(), event = ?event, "A tenant's speech cap passed a threshold");
        state
            .publish_event(fcp_events::PlatformEvent::Speech(event.clone()))
            .await;
        raised.push(event);
    }
    raised
}

/// Tell each tenant's sites its caps: for every tenant with a site and every
/// provider capped for it (never the default), the cap that applies and the
/// month's synthesized characters by where they were spoken, kept in
/// `site_speech_caps`, which the journal carries down (`docs/SPEECH_CAPS.md`
/// §5). A cap gone is removed; a row is written only when it changed.
async fn carry_to_sites(
    state: &ApiState,
    caps: &[SpeechCap],
    month: std::time::SystemTime,
    default: Option<&str>,
) {
    use fcp_core::site::SiteStore;
    use fcp_core::speech::SiteSpeechCap;
    let (Some(storage), Ok(store)) = (state.storage.as_ref(), usage_store(state)) else {
        return;
    };
    let sites = match storage.list_all_sites().await {
        Ok(sites) => sites,
        Err(e) => {
            tracing::warn!(error = %e, "Speech cap sweep: sites could not be listed");
            return;
        }
    };
    let mut tenants: Vec<fcp_core::TenantId> = sites.iter().map(|s| s.tenant_id).collect();
    tenants.sort_by_key(|t| t.to_string());
    tenants.dedup();
    if tenants.is_empty() {
        return;
    }
    let by_source = if caps.is_empty() {
        Vec::new()
    } else {
        match store.synthesized_by_source(month).await {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!(error = %e, "Speech cap sweep: usage by source could not be read");
                return;
            }
        }
    };
    for tenant in tenants {
        let mut providers: Vec<&str> = caps
            .iter()
            .filter(|c| c.tenant_id.is_none() || c.tenant_id == Some(tenant))
            .map(|c| c.provider.as_str())
            .filter(|p| Some(*p) != default)
            .collect();
        providers.sort_unstable();
        providers.dedup();
        let mut kept: Vec<String> = Vec::new();
        for provider in providers {
            let Some(cap) = effective_cap(caps, &tenant, provider) else {
                continue;
            };
            let used = by_source
                .iter()
                .filter(|(t, p, _, _)| *t == tenant && p == provider)
                .map(|(_, _, source, n)| (source.clone(), *n))
                .collect();
            let row = SiteSpeechCap {
                tenant_id: tenant,
                provider: provider.to_string(),
                month,
                cap,
                used,
            };
            if let Err(e) = store.put_site_speech_cap(&row).await {
                tracing::warn!(error = %e, "Speech cap sweep: a site's cap could not be kept");
            }
            kept.push(provider.to_string());
        }
        if let Ok(rows) = store.site_speech_caps(&tenant).await {
            for row in rows.into_iter().filter(|r| !kept.contains(&r.provider)) {
                let _ = store.delete_site_speech_cap(&tenant, &row.provider).await;
            }
        }
    }
}

/// Sweep the caps every [`CAP_SWEEP_EVERY`], one admin at a time.
pub fn spawn_cap_sweep(state: ApiState) {
    tokio::spawn(async move {
        use fcp_core::storage::CdrStore;
        let mut tick = tokio::time::interval(CAP_SWEEP_EVERY);
        loop {
            tick.tick().await;
            let Some(storage) = state.storage.clone() else {
                continue;
            };
            let lock = match storage.try_maintenance_lock("fcp:speech-caps").await {
                Ok(Some(lock)) => lock,
                Ok(None) => continue,
                Err(e) => {
                    tracing::warn!(error = %e, "Speech cap sweep: could not take the lock");
                    continue;
                }
            };
            sweep_caps(&state, std::time::SystemTime::now()).await;
            lock.release().await;
        }
    });
}
