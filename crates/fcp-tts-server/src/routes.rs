use axum::{
    extract::{Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{info, warn};

use crate::state::AppState;

// ---------------------------------------------------------------------------
// POST /v1/synthesize
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct SynthesizeRequest {
    pub text: String,
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default)]
    pub voice: Option<String>,
    /// Optional provider name override (uses default if omitted).
    #[serde(default)]
    pub provider: Option<String>,
    /// Tenant ID for usage tracking.
    #[serde(default)]
    pub tenant_id: Option<String>,
}

fn default_language() -> String {
    "en-US".to_string()
}

pub async fn synthesize(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SynthesizeRequest>,
) -> Response {
    let provider_name = req.provider.as_deref();
    let service = match state.service(provider_name) {
        Some(s) => s,
        None => {
            let name = provider_name.unwrap_or(&state.default_provider);
            warn!(provider = %name, "Provider not found");
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("unknown provider '{}'", name)})),
            )
                .into_response();
        }
    };

    let voice = req.voice.as_deref().or(state.config.tts.default_voice.as_deref());

    match service.synthesize(&req.text, Some(&req.language), voice).await {
        Ok(path) => {
            let chars = req.text.len();
            info!(
                text_len = chars,
                language = %req.language,
                voice = ?voice,
                provider = provider_name.unwrap_or(&state.default_provider),
                tenant_id = ?req.tenant_id,
                cached_path = %path.display(),
                "Synthesis complete"
            );

            // Read the WAV file and return as bytes.
            match tokio::fs::read(&path).await {
                Ok(bytes) => (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "audio/wav")],
                    bytes,
                )
                    .into_response(),
                Err(e) => {
                    warn!(error = %e, path = %path.display(), "Failed to read synthesized audio");
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({"error": "failed to read audio file"})),
                    )
                        .into_response()
                }
            }
        }
        Err(e) => {
            warn!(error = %e, "Synthesis failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("{}", e)})),
            )
                .into_response()
        }
    }
}

// ---------------------------------------------------------------------------
// GET /v1/voices
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct VoicesQuery {
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default = "default_language")]
    pub language: String,
}

pub async fn list_voices(
    State(state): State<Arc<AppState>>,
    Query(q): Query<VoicesQuery>,
) -> Response {
    let provider_name = q
        .provider
        .as_deref()
        .unwrap_or(&state.default_provider);

    // Return the configured provider info — voice listing is provider-specific
    // and not all providers expose a list_voices API on TtsService.
    let voices = state
        .config
        .tts
        .providers
        .iter()
        .filter(|p| p.name == provider_name)
        .map(|p| {
            serde_json::json!({
                "provider": p.name,
                "type": p.provider_type,
                "language": q.language,
            })
        })
        .collect::<Vec<_>>();

    (StatusCode::OK, Json(voices)).into_response()
}

// ---------------------------------------------------------------------------
// GET /v1/providers
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct ProviderInfo {
    name: String,
    is_default: bool,
}

pub async fn list_providers(State(state): State<Arc<AppState>>) -> Json<Vec<ProviderInfo>> {
    let list: Vec<ProviderInfo> = state
        .services
        .keys()
        .map(|name| ProviderInfo {
            name: name.clone(),
            is_default: name == &state.default_provider,
        })
        .collect();
    Json(list)
}

// ---------------------------------------------------------------------------
// GET /v1/cache/stats
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct CacheStats {
    entry_count: usize,
    size_bytes: usize,
    max_size_mb: usize,
}

pub async fn cache_stats(State(state): State<Arc<AppState>>) -> Json<CacheStats> {
    let (count, size) = if let Some(svc) = state.default_service() {
        (svc.cache_entry_count().await, svc.cache_size_bytes().await)
    } else {
        (0, 0)
    };
    Json(CacheStats {
        entry_count: count,
        size_bytes: size,
        max_size_mb: state.config.tts.cache_max_size_mb,
    })
}

// ---------------------------------------------------------------------------
// DELETE /v1/cache
// ---------------------------------------------------------------------------

pub async fn clear_cache(State(state): State<Arc<AppState>>) -> StatusCode {
    for svc in state.services.values() {
        let _ = svc.clear_cache().await;
    }
    info!("TTS cache cleared");
    StatusCode::NO_CONTENT
}

// ---------------------------------------------------------------------------
// GET /health
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct HealthResponse {
    service: &'static str,
    status: &'static str,
    providers: Vec<String>,
    cache_entries: usize,
}

pub async fn health(State(state): State<Arc<AppState>>) -> Json<HealthResponse> {
    let providers: Vec<String> = state.services.keys().cloned().collect();
    let cache_entries = if let Some(svc) = state.default_service() {
        svc.cache_entry_count().await
    } else {
        0
    };
    Json(HealthResponse {
        service: "fcp-tts",
        status: if state.services.is_empty() {
            "degraded"
        } else {
            "healthy"
        },
        providers,
        cache_entries,
    })
}
