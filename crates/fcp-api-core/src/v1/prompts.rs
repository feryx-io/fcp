//! Audio prompt management API endpoints.

use crate::{auth::TenantContext, error::ApiError, state::ApiState};
use axum::{
    extract::{Multipart, Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use fcp_core::prompt::{Prompt, PromptCategory, PromptSourceType, PromptStore};
use fcp_core::PromptId;
use serde::{Deserialize, Serialize};
use std::time::SystemTime;
use tracing::info;

/// Helper to get prompt store from DynStorage.
fn require_prompt_store(state: &ApiState) -> Result<&std::sync::Arc<dyn PromptStore>, ApiError> {
    let storage = state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::Internal("Storage not configured".to_string()))?;
    storage
        .prompt_store()
        .ok_or_else(|| ApiError::Internal("Prompt store not configured".to_string()))
}

fn system_time_to_epoch_secs(st: SystemTime) -> f64 {
    st.duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

fn ensure_prompt_tenant(
    prompt: Prompt,
    tenant_id: &fcp_core::TenantId,
    id: &str,
) -> Result<Prompt, ApiError> {
    if prompt.tenant_id != *tenant_id {
        return Err(ApiError::NotFound(format!("Prompt {} not found", id)));
    }
    Ok(prompt)
}

// ── Response types ────────────────────────────────────────────────────

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct PromptResponse {
    pub id: String,
    pub tenant_id: String,
    pub name: String,
    pub description: Option<String>,
    pub category: String,
    pub language: String,
    pub source_type: String,
    pub tts_text: Option<String>,
    pub tts_voice: Option<String>,
    pub audio_path: Option<String>,
    pub audio_format: Option<String>,
    pub duration_ms: Option<u32>,
    pub file_size: Option<u64>,
    pub is_system: bool,
    pub has_audio: bool,
    #[serde(with = "crate::time::epoch_secs_f64")]
    #[schema(value_type = String, format = DateTime)]
    pub created_at: f64,
    #[serde(with = "crate::time::epoch_secs_f64")]
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: f64,
}

impl From<Prompt> for PromptResponse {
    fn from(p: Prompt) -> Self {
        let has_audio = p
            .audio_path
            .as_ref()
            .map(|path| path.exists())
            .unwrap_or(false);
        Self {
            id: p.id.to_string(),
            tenant_id: p.tenant_id.to_string(),
            name: p.name,
            description: p.description,
            category: p.category.to_string(),
            language: p.language,
            source_type: p.source_type.to_string(),
            tts_text: p.tts_text,
            tts_voice: p.tts_voice,
            audio_path: p.audio_path.map(|p| p.to_string_lossy().to_string()),
            audio_format: p.audio_format,
            duration_ms: p.duration_ms,
            file_size: p.file_size,
            is_system: p.is_system,
            has_audio,
            created_at: system_time_to_epoch_secs(p.created_at),
            updated_at: system_time_to_epoch_secs(p.updated_at),
        }
    }
}

// ── Request types ─────────────────────────────────────────────────────

#[derive(Debug, Deserialize, utoipa::ToSchema, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListPromptsQuery {
    pub category: Option<String>,
    pub language: Option<String>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreatePromptRequest {
    pub name: String,
    pub description: Option<String>,
    pub category: Option<String>,
    pub language: Option<String>,
    pub source_type: Option<String>,
    pub tts_text: Option<String>,
    pub tts_voice: Option<String>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdatePromptRequest {
    pub description: Option<String>,
    pub category: Option<String>,
    pub language: Option<String>,
    pub tts_text: Option<String>,
    pub tts_voice: Option<String>,
}

// ── Handlers ──────────────────────────────────────────────────────────

/// GET /api/v1/prompts
#[utoipa::path(
    get,
    path = "/api/v1/prompts",
    tag = "prompts",
    operation_id = "prompts_list_prompts",
    params(ListPromptsQuery, crate::page::PageQuery),
    responses(
        (status = 200, description = "OK", body = crate::page::Page<PromptResponse>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_prompts(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Query(query): Query<ListPromptsQuery>,
    axum::extract::Query(page): axum::extract::Query<crate::page::PageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let prompt_store = require_prompt_store(&state)?;

    let category = match &query.category {
        Some(c) => Some(
            c.parse::<PromptCategory>()
                .map_err(|e| ApiError::BadRequest(e.to_string()))?,
        ),
        None => None,
    };

    let prompts = prompt_store
        .list_prompts(&tenant.0, category, query.language.as_deref())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let responses: Vec<PromptResponse> = prompts.into_iter().map(PromptResponse::from).collect();
    Ok(Json(crate::page::paginate(
        responses,
        &page,
        crate::page::Order::Ascending,
        |r| format!("{}/{}", r.name, r.id),
    )?))
}

/// GET /api/v1/prompts/:id
#[utoipa::path(
    get,
    path = "/api/v1/prompts/{id}",
    tag = "prompts",
    operation_id = "prompts_get_prompt",
    params(("id" = String, Path)),
    responses(
        (status = 200, description = "OK", body = PromptResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn get_prompt(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let prompt_store = require_prompt_store(&state)?;

    let prompt_id: PromptId = id
        .parse()
        .map_err(|_| ApiError::BadRequest("Invalid prompt ID".to_string()))?;

    let prompt = ensure_prompt_tenant(
        prompt_store
            .get_prompt(&prompt_id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
            .ok_or_else(|| ApiError::NotFound(format!("Prompt {} not found", id)))?,
        &tenant.0,
        &id,
    )?;

    Ok(Json(PromptResponse::from(prompt)))
}

/// POST /api/v1/prompts
#[utoipa::path(
    post,
    path = "/api/v1/prompts",
    tag = "prompts",
    operation_id = "prompts_create_prompt",
    request_body = CreatePromptRequest,
    responses(
        (status = 201, description = "OK", body = PromptResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn create_prompt(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Json(req): Json<CreatePromptRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let prompt_store = require_prompt_store(&state)?;

    if req.name.is_empty() {
        return Err(ApiError::BadRequest("Name is required".to_string()));
    }

    let category = req
        .category
        .as_deref()
        .unwrap_or("custom")
        .parse::<PromptCategory>()
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    let language = req.language.as_deref().unwrap_or("en-US");

    let source_type = req
        .source_type
        .as_deref()
        .unwrap_or("tts")
        .parse::<PromptSourceType>()
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    let mut prompt = match source_type {
        PromptSourceType::Tts => {
            let tts_text = req.tts_text.ok_or_else(|| {
                ApiError::BadRequest("tts_text is required for TTS prompts".to_string())
            })?;
            Prompt::new_tts(tenant.0, &req.name, category, language, &tts_text)
        }
        PromptSourceType::File => Prompt::new_file(
            tenant.0,
            &req.name,
            category,
            language,
            std::path::PathBuf::new(), // placeholder — audio uploaded separately
        ),
    };

    prompt.description = req.description;
    prompt.tts_voice = req.tts_voice.clone();

    // Auto-synthesize TTS audio if TTS service is available
    if source_type == PromptSourceType::Tts {
        if let (Some(tts_service), Some(prompts_dir)) = (&state.tts_service, &state.prompts_dir) {
            let tts_text = prompt.tts_text.as_deref().unwrap_or("");
            let voice = req.tts_voice.as_deref();
            match tts_service
                .synthesize(tts_text, Some(language), voice)
                .await
            {
                Ok(cached_path) => {
                    let tenant_dir = prompts_dir.join(prompt.tenant_id.to_string());
                    let _ = tokio::fs::create_dir_all(&tenant_dir).await;
                    let dest_path = tenant_dir.join(format!("{}.wav", prompt.id));
                    if tokio::fs::copy(&cached_path, &dest_path).await.is_ok() {
                        let file_size = tokio::fs::metadata(&dest_path)
                            .await
                            .map(|m| m.len())
                            .unwrap_or(0);
                        prompt.audio_path = Some(dest_path);
                        prompt.audio_format = Some("wav".to_string());
                        prompt.file_size = Some(file_size);
                        info!(name = %req.name, "Auto-synthesized TTS audio for new prompt");
                    }
                }
                Err(e) => {
                    tracing::warn!(name = %req.name, error = %e, "Auto-synthesis failed (prompt created without audio)");
                }
            }
        }
    }

    prompt_store
        .put_prompt(prompt.clone())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    info!(name = %req.name, category = %category, "Created prompt");

    Ok((StatusCode::CREATED, Json(PromptResponse::from(prompt))))
}

/// PUT /api/v1/prompts/:id
#[utoipa::path(
    put,
    path = "/api/v1/prompts/{id}",
    tag = "prompts",
    operation_id = "prompts_update_prompt",
    params(("id" = String, Path)),
    request_body = UpdatePromptRequest,
    responses(
        (status = 200, description = "OK", body = PromptResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn update_prompt(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(id): Path<String>,
    Json(req): Json<UpdatePromptRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let prompt_store = require_prompt_store(&state)?;

    let prompt_id: PromptId = id
        .parse()
        .map_err(|_| ApiError::BadRequest("Invalid prompt ID".to_string()))?;

    let mut prompt = ensure_prompt_tenant(
        prompt_store
            .get_prompt(&prompt_id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
            .ok_or_else(|| ApiError::NotFound(format!("Prompt {} not found", id)))?,
        &tenant.0,
        &id,
    )?;

    // Track if TTS parameters changed (invalidates cached audio)
    let tts_changed = (req.tts_text.is_some() && req.tts_text != prompt.tts_text)
        || (req.tts_voice.is_some() && req.tts_voice != prompt.tts_voice)
        || (req.language.is_some() && req.language.as_deref() != Some(&prompt.language));

    if let Some(desc) = req.description {
        prompt.description = Some(desc);
    }
    if let Some(cat) = req.category {
        prompt.category = cat
            .parse()
            .map_err(|e: anyhow::Error| ApiError::BadRequest(e.to_string()))?;
    }
    if let Some(lang) = req.language {
        prompt.language = lang;
    }
    if let Some(text) = req.tts_text {
        prompt.tts_text = Some(text);
    }
    if let Some(voice) = req.tts_voice {
        prompt.tts_voice = Some(voice);
    }

    // New words are spoken before anything is saved: a prompt whose text
    // changed and could not be spoken again is refused as it was, rather
    // than kept with no audio, which a call would play as silence.
    if tts_changed && prompt.source_type == PromptSourceType::Tts {
        let (Some(tts_service), Some(prompts_dir)) = (&state.tts_service, &state.prompts_dir)
        else {
            return Err(ApiError::ServiceUnavailable(
                "TTS service not configured: the new text cannot be spoken".to_string(),
            ));
        };
        let tts_text = prompt.tts_text.as_deref().unwrap_or("");
        let voice = prompt.tts_voice.as_deref();
        let cached_path = tts_service
            .synthesize(tts_text, Some(&prompt.language), voice)
            .await
            .map_err(|e| ApiError::ServiceUnavailable(format!("TTS synthesis failed: {e}")))?;
        let tenant_dir = prompts_dir.join(prompt.tenant_id.to_string());
        tokio::fs::create_dir_all(&tenant_dir)
            .await
            .map_err(|e| ApiError::Internal(format!("Failed to create prompts directory: {e}")))?;
        let dest_path = tenant_dir.join(format!("{}.wav", prompt.id));
        tokio::fs::copy(&cached_path, &dest_path)
            .await
            .map_err(|e| ApiError::Internal(format!("Failed to save audio file: {e}")))?;
        prompt.file_size = Some(
            tokio::fs::metadata(&dest_path)
                .await
                .map(|m| m.len())
                .unwrap_or(0),
        );
        prompt.audio_path = Some(dest_path);
        prompt.audio_format = Some("wav".to_string());
        prompt.duration_ms = None;
        info!(id = %id, "Re-synthesized TTS audio after text change");
    }

    prompt.updated_at = SystemTime::now();

    prompt_store
        .put_prompt(prompt.clone())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    info!(id = %id, "Updated prompt");

    Ok(Json(PromptResponse::from(prompt)))
}

/// POST /api/v1/prompts/:id/audio — Upload/replace audio WAV file
#[utoipa::path(
    post,
    path = "/api/v1/prompts/{id}/audio",
    tag = "prompts",
    operation_id = "prompts_upload_audio",
    params(("id" = String, Path)),
    request_body(content = crate::v1::uploads::PromptAudioUpload, content_type = "multipart/form-data"),
    responses(
        (status = 200, description = "OK", body = PromptResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn upload_audio(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(id): Path<String>,
    mut multipart: Multipart,
) -> Result<impl IntoResponse, ApiError> {
    let prompt_store = require_prompt_store(&state)?;

    let prompts_dir = state
        .prompts_dir
        .as_ref()
        .ok_or_else(|| ApiError::Internal("Prompts directory not configured".to_string()))?;

    let prompt_id: PromptId = id
        .parse()
        .map_err(|_| ApiError::BadRequest("Invalid prompt ID".to_string()))?;

    let mut prompt = ensure_prompt_tenant(
        prompt_store
            .get_prompt(&prompt_id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
            .ok_or_else(|| ApiError::NotFound(format!("Prompt {} not found", id)))?,
        &tenant.0,
        &id,
    )?;

    // Read audio data from multipart
    let mut audio_data: Option<Vec<u8>> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::BadRequest(format!("Failed to read multipart field: {}", e)))?
    {
        let name = field.name().unwrap_or("").to_string();
        if name == "audio" {
            let bytes = field
                .bytes()
                .await
                .map_err(|e| ApiError::BadRequest(format!("Failed to read audio data: {}", e)))?;
            audio_data = Some(bytes.to_vec());
        }
    }

    let audio_data =
        audio_data.ok_or_else(|| ApiError::BadRequest("Missing 'audio' file field".to_string()))?;
    if audio_data.is_empty() {
        return Err(ApiError::BadRequest("Audio file is empty".to_string()));
    }

    // Create directory: prompts_dir/{tenant_id}/
    let tenant_dir = prompts_dir.join(prompt.tenant_id.to_string());
    tokio::fs::create_dir_all(&tenant_dir)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to create prompts directory: {}", e)))?;

    // Write file: prompts_dir/{tenant_id}/{prompt_id}.wav
    let file_path = tenant_dir.join(format!("{}.wav", prompt.id));
    tokio::fs::write(&file_path, &audio_data)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to write audio file: {}", e)))?;

    // Update prompt record
    prompt.audio_path = Some(file_path);
    prompt.audio_format = Some("wav".to_string());
    prompt.file_size = Some(audio_data.len() as u64);
    prompt.source_type = PromptSourceType::File;
    prompt.updated_at = SystemTime::now();

    prompt_store
        .put_prompt(prompt.clone())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    info!(id = %id, size = audio_data.len(), "Uploaded prompt audio");

    Ok((StatusCode::OK, Json(PromptResponse::from(prompt))))
}

/// GET /api/v1/prompts/:id/audio — Download/stream audio file
#[utoipa::path(
    get,
    path = "/api/v1/prompts/{id}/audio",
    tag = "prompts",
    operation_id = "prompts_download_audio",
    params(("id" = String, Path)),
    responses(
        (status = 200, description = "The audio", content_type = "audio/wav", body = Vec<u8>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn download_audio(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let prompt_store = require_prompt_store(&state)?;

    let prompt_id: PromptId = id
        .parse()
        .map_err(|_| ApiError::BadRequest("Invalid prompt ID".to_string()))?;

    let prompt = ensure_prompt_tenant(
        prompt_store
            .get_prompt(&prompt_id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
            .ok_or_else(|| ApiError::NotFound(format!("Prompt {} not found", id)))?,
        &tenant.0,
        &id,
    )?;

    let audio_path = prompt
        .audio_path
        .ok_or_else(|| ApiError::NotFound("No audio file for this prompt".to_string()))?;

    let data = tokio::fs::read(&audio_path)
        .await
        .map_err(|e| ApiError::NotFound(format!("Audio file not found: {}", e)))?;

    Ok((
        StatusCode::OK,
        [
            (axum::http::header::CONTENT_TYPE, "audio/wav"),
            (axum::http::header::CONTENT_DISPOSITION, "inline"),
        ],
        data,
    ))
}

/// POST /api/v1/prompts/:id/synthesize — Generate TTS audio for a prompt
#[utoipa::path(
    post,
    path = "/api/v1/prompts/{id}/synthesize",
    tag = "prompts",
    operation_id = "prompts_synthesize_prompt",
    params(("id" = String, Path)),
    responses(
        (status = 200, description = "OK", body = PromptResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn synthesize_prompt(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let prompt_store = require_prompt_store(&state)?;

    let prompts_dir = state
        .prompts_dir
        .as_ref()
        .ok_or_else(|| ApiError::Internal("Prompts directory not configured".to_string()))?;

    let tts_service = state
        .tts_service
        .as_ref()
        .ok_or_else(|| ApiError::ServiceUnavailable("TTS service not configured".to_string()))?;

    let prompt_id: PromptId = id
        .parse()
        .map_err(|_| ApiError::BadRequest("Invalid prompt ID".to_string()))?;

    let mut prompt = ensure_prompt_tenant(
        prompt_store
            .get_prompt(&prompt_id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
            .ok_or_else(|| ApiError::NotFound(format!("Prompt {} not found", id)))?,
        &tenant.0,
        &id,
    )?;

    let tts_text = prompt
        .tts_text
        .as_ref()
        .ok_or_else(|| ApiError::BadRequest("Prompt has no TTS text to synthesize".to_string()))?
        .clone();

    let voice = prompt.tts_voice.as_deref();
    let language = &prompt.language;

    // Synthesize via TTS service
    let cached_path = tts_service
        .synthesize(&tts_text, Some(language), voice)
        .await
        .map_err(|e| ApiError::ServiceUnavailable(format!("TTS synthesis failed: {}", e)))?;

    // Copy to permanent prompts directory
    let tenant_dir = prompts_dir.join(prompt.tenant_id.to_string());
    tokio::fs::create_dir_all(&tenant_dir)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to create prompts directory: {}", e)))?;

    let dest_path = tenant_dir.join(format!("{}.wav", prompt.id));
    tokio::fs::copy(&cached_path, &dest_path)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to save audio file: {}", e)))?;

    let file_size = tokio::fs::metadata(&dest_path)
        .await
        .map(|m| m.len())
        .unwrap_or(0);

    // Update prompt record
    prompt.audio_path = Some(dest_path);
    prompt.audio_format = Some("wav".to_string());
    prompt.file_size = Some(file_size);
    prompt.updated_at = SystemTime::now();

    prompt_store
        .put_prompt(prompt.clone())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    info!(id = %id, name = %prompt.name, size = file_size, "Synthesized prompt audio via TTS");

    Ok(Json(PromptResponse::from(prompt)))
}

/// DELETE /api/v1/prompts/:id
#[utoipa::path(
    delete,
    path = "/api/v1/prompts/{id}",
    tag = "prompts",
    operation_id = "prompts_delete_prompt",
    params(("id" = String, Path)),
    responses(
        (status = 204, description = "Done"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn delete_prompt(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let prompt_store = require_prompt_store(&state)?;

    let prompt_id: PromptId = id
        .parse()
        .map_err(|_| ApiError::BadRequest("Invalid prompt ID".to_string()))?;

    // Check if prompt exists and is not a system prompt
    let prompt = ensure_prompt_tenant(
        prompt_store
            .get_prompt(&prompt_id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
            .ok_or_else(|| ApiError::NotFound(format!("Prompt {} not found", id)))?,
        &tenant.0,
        &id,
    )?;

    if prompt.is_system {
        return Err(ApiError::Forbidden(
            "Cannot delete system prompts".to_string(),
        ));
    }

    // Delete audio file if it exists
    if let Some(ref audio_path) = prompt.audio_path {
        let _ = tokio::fs::remove_file(audio_path).await;
    }

    prompt_store
        .delete_prompt(&prompt_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    info!(id = %id, name = %prompt.name, "Deleted prompt");

    Ok(StatusCode::NO_CONTENT)
}
