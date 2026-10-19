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

    let voice = req
        .voice
        .as_deref()
        .or(state.config.tts.default_voice.as_deref());

    match service
        .synthesize(&req.text, Some(&req.language), voice)
        .await
    {
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
                Ok(bytes) => {
                    (StatusCode::OK, [(header::CONTENT_TYPE, "audio/wav")], bytes).into_response()
                }
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
    let provider_name = q.provider.as_deref();
    let service = match state.service(provider_name) {
        Some(service) => service,
        None => {
            let name = provider_name.unwrap_or(&state.default_provider);
            warn!(provider = %name, "Provider not found for voice listing");
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("unknown provider '{}'", name)})),
            )
                .into_response();
        }
    };

    match service.list_voices(&q.language).await {
        Ok(mut voices) => {
            let logical_provider = provider_name.unwrap_or(&state.default_provider).to_string();
            for voice in &mut voices {
                voice
                    .metadata
                    .entry("logical_provider".to_string())
                    .or_insert_with(|| logical_provider.clone());
            }

            (StatusCode::OK, Json(voices)).into_response()
        }
        Err(error) => {
            let name = provider_name.unwrap_or(&state.default_provider);
            warn!(provider = %name, error = %error, "Failed to list provider voices");
            (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({"error": format!("failed to list voices for provider '{}'", name)})),
            )
                .into_response()
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    async fn test_state() -> Arc<AppState> {
        let cache_dir =
            std::env::temp_dir().join(format!("fcp-tts-routes-test-{}", uuid::Uuid::new_v4()));
        let service = Arc::new(
            fcp_media::tts::TtsService::new(fcp_media::tts::TtsConfig {
                provider: fcp_media::tts::TtsProviderConfig::OpenAi {
                    api_key: "test-key".to_string(),
                    model: "tts-1".to_string(),
                },
                cache_dir,
                cache_max_size_mb: 100,
                default_language: "en-US".to_string(),
                default_voice: None,
            })
            .await
            .unwrap(),
        );

        let mut services = std::collections::HashMap::new();
        services.insert("primary".to_string(), service);

        Arc::new(AppState {
            config: crate::config::TtsServerConfig {
                service: crate::config::ServiceSection::default(),
                http: crate::config::HttpSection::default(),
                auth: crate::config::AuthSection {
                    tokens: vec!["test-token".to_string()],
                    allow_unauthenticated: false,
                },
                tts: crate::config::TtsSection {
                    default_provider: "primary".to_string(),
                    cache_dir: std::env::temp_dir()
                        .join(format!("fcp-tts-routes-config-{}", uuid::Uuid::new_v4())),
                    cache_max_size_mb: 100,
                    default_language: "en-US".to_string(),
                    default_voice: None,
                    providers: vec![crate::config::ProviderEntry {
                        name: "primary".to_string(),
                        provider_type: "openai".to_string(),
                        engine: None,
                        python_path: None,
                        api_key: Some("test-key".to_string()),
                        model: Some("tts-1".to_string()),
                        device: "auto".to_string(),
                        binary: None,
                        voices_dir: None,
                        voice: None,
                    }],
                },
                stt: crate::config::SttSection::default(),
                logging: crate::config::LoggingSection::default(),
                cluster: crate::config::ClusterSection::default(),
            },
            services,
            default_provider: "primary".to_string(),
            stt: std::collections::HashMap::new(),
            stt_default: None,
        })
    }

    #[tokio::test]
    async fn test_synthesize_rejects_unknown_provider() {
        let response = synthesize(
            State(test_state().await),
            Json(SynthesizeRequest {
                text: "hello".to_string(),
                language: "en-US".to_string(),
                voice: None,
                provider: Some("missing".to_string()),
                tenant_id: None,
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"], "unknown provider 'missing'");
    }

    #[tokio::test]
    async fn test_list_voices_returns_real_provider_voices() {
        let response = list_voices(
            State(test_state().await),
            Query(VoicesQuery {
                provider: None,
                language: "en-US".to_string(),
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let voices: Vec<fcp_media::tts::VoiceInfo> = serde_json::from_slice(&body).unwrap();
        assert!(!voices.is_empty());
        assert!(voices.iter().any(|voice| voice.name == "alloy"));
        assert!(voices
            .iter()
            .all(|voice| voice.metadata.get("logical_provider") == Some(&"primary".to_string())));
    }
}
