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
    let provider = checked("provider", req.provider)?;
    known_provider(&state, &identity, &tenant.0, provider.as_deref()).await?;
    let speech = TenantSpeech {
        tenant_id: tenant.0,
        provider,
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
    /// What names it in the settings and requests (`account:<id>` for one
    /// of the tenant's accounts).
    pub name: String,
    /// What to show: the provider's name, or the account's.
    pub label: String,
    /// The TTS server's default: what a tenant without a choice gets.
    pub is_default: bool,
    /// One of the tenant's own accounts (`docs/TENANT_SPEECH.md` S3).
    pub account: bool,
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
        })
        .collect();
    list.sort_by(|a, b| b.is_default.cmp(&a.is_default).then(a.name.cmp(&b.name)));
    // A tenant sees what the platform offers it; the default always is.
    if !identity.is_platform_admin() {
        let held = held_back(&state).await?;
        list.retain(|p| p.is_default || !held.contains(&p.name));
    }
    // Then the tenant's own accounts, by name.
    if let Ok(store) = accounts(&state) {
        let mine = store
            .list_speech_accounts(&tenant.0)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        list.extend(mine.into_iter().map(|a| SpeechProvider {
            name: format!("{ACCOUNT_PREFIX}{}", a.id),
            label: a.name,
            is_default: false,
            account: true,
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
    known_provider(&state, &identity, &tenant.0, provider.as_deref()).await?;
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
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn test_account(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: TenantContext,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<impl axum::response::IntoResponse, ApiError> {
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
        .await
        .map_err(|e| ApiError::ServiceUnavailable(format!("The account could not speak: {e:#}")))?;
    Ok((
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "audio/wav")],
        audio,
    ))
}

// ── The platform's offer (docs/TENANT_SPEECH.md S3) ────────────────────────

/// One of the TTS server's providers, as the platform offers it.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct OfferedProvider {
    pub name: String,
    pub is_default: bool,
    /// Whether tenants may choose it.
    pub offered: bool,
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
}

fn require_platform(identity: &AuthIdentity) -> Result<(), ApiError> {
    if identity.is_platform_admin() {
        Ok(())
    } else {
        Err(ApiError::Forbidden("Platform admin access required".into()))
    }
}

async fn platform_speech(state: &ApiState) -> Result<PlatformSpeech, ApiError> {
    let held_back = held_back(state).await?;
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
                offered: p.is_default || !held_back.contains(&p.name),
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
                .any(|p| p.is_default && held.contains(&p.name))
            {
                return Err(ApiError::BadRequest(
                    "The default provider speaks for tenants that choose none: it cannot be held back"
                        .into(),
                ));
            }
        }
    }
    store(&state)?
        .put_held_back_providers(&held)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
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
    let names: std::collections::HashMap<String, String> = match accounts(&state) {
        Ok(store) => store
            .list_speech_accounts(&tenant.0)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
            .into_iter()
            .map(|a| (format!("{ACCOUNT_PREFIX}{}", a.id), a.name))
            .collect(),
        Err(_) => Default::default(),
    };
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
            });
        p.requests += h.requests;
        p.characters += h.characters;
        p.synthesized_characters += h.synthesized_characters;
    }
    let list: Vec<SpeechUsagePeriod> = periods.into_values().collect();
    let total = list.len() as u64;
    Ok(Json(crate::page::Page::new(list, None, Some(total))))
}
