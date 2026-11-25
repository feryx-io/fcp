//! fcp-tts — Dedicated shared Text-to-Speech service.
//!
//! Centralizes TTS synthesis so that call-manager, conference, IVR, and admin
//! services share a single Kokoro/cloud TTS backend instead of each spawning
//! their own subprocess.

mod cli;
mod config;
mod routes;
mod state;
mod transcribe;

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::{
    extract::{DefaultBodyLimit, Request},
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Router,
};
use clap::Parser;
use tower_http::cors::CorsLayer;
use tracing::{info, warn};

use crate::config::TtsServerConfig;
use crate::state::AppState;

#[tokio::main]
async fn main() -> Result<()> {
    // 1. CLI
    let cli = cli::Cli::parse();

    // 2. Config
    let mut config = fcp_core::config::check_stage(
        config::load_config(cli.config.as_deref()),
        cli.check_config,
    )?;
    // Secrets from the environment win over the file
    // (`docs/SHIP_AND_OPERATE.md` phase 2).
    if let Some(v) = &cli.auth_tokens {
        config.auth.tokens = split_tokens(v);
    }
    if let Some(ref level) = cli.log_level {
        config.logging.level = level.clone();
    }
    if let Some(ref fmt) = cli.log_format {
        config.logging.format = fmt.clone();
    }
    if let Some(ref bind) = cli.bind {
        config.http.bind_address = bind.clone();
    }
    fcp_core::config::check_stage(config::validate_runtime_config(&config), cli.check_config)?;

    // 3. Tracing
    if cli.check_config {
        println!(
            "{}",
            fcp_core::config::masked_toml(&config).map_err(anyhow::Error::msg)?
        );
        return Ok(());
    }
    init_tracing(&config);

    info!(
        version = env!("CARGO_PKG_VERSION"),
        node_id = %config.service.node_id,
        bind = %config.http.bind_address,
        providers = config.tts.providers.len(),
        speech_providers = config.stt.providers.len(),
        "fcp-tts starting"
    );

    // 4. Create TTS providers
    let mut services: HashMap<String, Arc<fcp_media::tts::TtsService>> = HashMap::new();

    for entry in &config.tts.providers {
        let provider_config = match build_provider_config(entry) {
            Ok(provider_config) => provider_config,
            Err(error) => {
                warn!(
                    provider = %entry.name,
                    error = %error,
                    "Invalid TTS provider configuration — skipping"
                );
                continue;
            }
        };
        let tts_config = fcp_media::tts::TtsConfig {
            provider: provider_config,
            cache_dir: config.tts.cache_dir.clone(),
            cache_max_size_mb: config.tts.cache_max_size_mb,
            default_language: config.tts.default_language.clone(),
            default_voice: config.tts.default_voice.clone(),
        };

        match fcp_media::tts::TtsService::new(tts_config).await {
            Ok(svc) => {
                info!(
                    provider = %entry.name,
                    provider_type = %entry.provider_type,
                    device = %entry.device,
                    "TTS provider initialized"
                );
                services.insert(entry.name.clone(), Arc::new(svc));
            }
            Err(e) => {
                warn!(
                    provider = %entry.name,
                    error = %e,
                    "Failed to initialize TTS provider — skipping"
                );
            }
        }
    }

    if !config.tts.providers.is_empty() {
        if services.is_empty() {
            anyhow::bail!("No TTS providers initialized");
        }

        if !services.contains_key(&config.tts.default_provider) {
            anyhow::bail!(
                "Default provider '{}' is not available after initialization",
                config.tts.default_provider
            );
        }
    }

    // Speech recognition: every named provider, or the server refuses to
    // start — a misconfigured recognizer is not skipped.
    let mut stt = HashMap::new();
    for (name, provider) in &config.stt.providers {
        let service = fcp_media::stt::SttService::new(fcp_media::stt::SttConfig {
            provider: provider.clone(),
            language: config.stt.language.clone(),
        })
        .await
        .with_context(|| format!("speech provider '{name}'"))?;
        info!(provider = %name, kind = %service.provider_name(), "speech provider initialized");
        stt.insert(name.clone(), Arc::new(service));
    }

    let app_state = Arc::new(AppState {
        default_provider: config.tts.default_provider.clone(),
        services,
        stt,
        stt_default: config.stt.default_name().map(String::from),
        config: config.clone(),
    });

    // 5. Axum router
    let auth_tokens = config.auth.tokens.clone();
    let app = Router::new()
        // Unprotected
        .route("/health", get(routes::health))
        // Protected
        .route("/v1/synthesize", post(routes::synthesize))
        .route("/v1/voices", get(routes::list_voices))
        .route("/v1/providers", get(routes::list_providers))
        .route("/v1/cache/stats", get(routes::cache_stats))
        .route("/v1/cache", delete(routes::clear_cache))
        .route(
            "/v1/transcribe",
            // A recording of up to half an hour at 16 kHz.
            post(transcribe::transcribe).layer(DefaultBodyLimit::max(64 * 1024 * 1024)),
        )
        .route("/v1/transcribe/stream", get(transcribe::transcribe_stream))
        .layer(middleware::from_fn(move |req, next| {
            let tokens = auth_tokens.clone();
            require_api_token(tokens, req, next)
        }))
        .layer(CorsLayer::permissive())
        .with_state(app_state);

    // 6. Cluster registration (optional)
    #[cfg(feature = "storage-redis")]
    if config.cluster.enabled {
        start_cluster(&config).await;
    }

    // Say this process is there, member or not (`docs/SERVICES.md`).
    #[cfg(feature = "storage-redis")]
    let beacon = fcp_cluster::ServiceBeacon::new(
        fcp_cluster::ServiceIdentity::new(&config.service.node_id, "tts")
            .region(fcp_cluster::resolve_region(
                config.cluster.region.as_deref(),
            ))
            .member(config.cluster.enabled)
            .http(&config.http.bind_address),
    )
    .start(fcp_storage_redis::RedisConfig {
        url: config.cluster.redis_url.clone(),
        ..Default::default()
    });

    // 7. Bind and serve
    let bind_addr: std::net::SocketAddr = config
        .http
        .bind_address
        .parse()
        .context("Invalid HTTP bind address")?;

    if config.auth.allow_unauthenticated && !bind_addr.ip().is_loopback() {
        warn!(
            bind = %bind_addr,
            "Unauthenticated mode is enabled on a non-loopback address"
        );
    }

    info!(
        bind = %bind_addr,
        default_provider = %config.tts.default_provider,
        "fcp-tts ready"
    );

    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    #[cfg(feature = "storage-redis")]
    beacon.shutdown().await;

    info!("fcp-tts shutdown complete");
    Ok(())
}

// ---------------------------------------------------------------------------
// Auth middleware
// ---------------------------------------------------------------------------

async fn require_api_token(tokens: Vec<String>, req: Request, next: Next) -> Response {
    // Skip auth for /health
    if req.uri().path() == "/health" {
        return next.run(req).await;
    }

    // No tokens configured = auth explicitly disabled
    if tokens.is_empty() {
        return next.run(req).await;
    }

    let auth_header = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok());

    if let Some(header) = auth_header {
        let token = extract_bearer_token(header).unwrap_or(header).trim();
        if tokens
            .iter()
            .any(|expected| fcp_core::secret::constant_time_eq(expected, token))
        {
            return next.run(req).await;
        }
    }

    (
        StatusCode::UNAUTHORIZED,
        "Invalid or missing authentication token",
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// Provider config builder
// ---------------------------------------------------------------------------

fn build_provider_config(
    entry: &config::ProviderEntry,
) -> anyhow::Result<fcp_media::tts::TtsProviderConfig> {
    if entry.name.trim().is_empty() {
        anyhow::bail!("provider name must not be empty");
    }

    match entry.provider_type.as_str() {
        "local" | "kokoro" => Ok(fcp_media::tts::TtsProviderConfig::Local {
            engine: entry.engine.clone().unwrap_or_else(|| "kokoro".to_string()),
            executable_path: entry.python_path.as_ref().map(std::path::PathBuf::from),
            device: Some(entry.device.clone()),
        }),
        "piper" => Ok(fcp_media::tts::TtsProviderConfig::Piper {
            binary: entry.binary.as_ref().map(std::path::PathBuf::from),
            voices_dir: std::path::PathBuf::from(require_non_empty(
                &entry.voices_dir,
                "voices_dir",
                entry,
            )?),
            voice: entry
                .voice
                .clone()
                .unwrap_or_else(|| "en_US-ljspeech-medium".to_string()),
        }),
        "openai" => Ok(fcp_media::tts::TtsProviderConfig::OpenAi {
            api_key: require_non_empty(&entry.api_key, "api_key", entry)?,
            model: entry.model.clone().unwrap_or_else(|| "tts-1".to_string()),
        }),
        "google_cloud" => Ok(fcp_media::tts::TtsProviderConfig::GoogleCloud {
            api_key: require_non_empty(&entry.api_key, "api_key", entry)?,
        }),
        "elevenlabs" => Ok(fcp_media::tts::TtsProviderConfig::ElevenLabs {
            api_key: require_non_empty(&entry.api_key, "api_key", entry)?,
            model_id: entry.model.clone(),
            endpoint: entry.endpoint.clone(),
        }),
        "azure" => Ok(fcp_media::tts::TtsProviderConfig::Azure {
            subscription_key: require_non_empty(&entry.api_key, "api_key", entry)?,
            region: require_non_empty(&entry.region, "region", entry)?,
            endpoint: entry.endpoint.clone(),
        }),
        "aws_polly" => Ok(fcp_media::tts::TtsProviderConfig::AwsPolly {
            region: require_non_empty(&entry.region, "region", entry)?,
            access_key_id: entry.access_key_id.clone(),
            secret_access_key: entry.secret_access_key.clone(),
            engine: entry.engine.clone().unwrap_or_else(|| "neural".to_string()),
            endpoint: entry.endpoint.clone(),
        }),
        other => anyhow::bail!("unsupported provider type '{}'", other),
    }
}

fn require_non_empty(
    value: &Option<String>,
    field_name: &str,
    entry: &config::ProviderEntry,
) -> anyhow::Result<String> {
    match value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(value) => Ok(value.to_string()),
        None => anyhow::bail!(
            "provider '{}' requires a non-empty {}",
            entry.name,
            field_name
        ),
    }
}

fn extract_bearer_token(header_value: &str) -> Option<&str> {
    header_value
        .strip_prefix("Bearer ")
        .or_else(|| header_value.strip_prefix("bearer "))
        .map(str::trim)
}

// ---------------------------------------------------------------------------
// Cluster
// ---------------------------------------------------------------------------

#[cfg(feature = "storage-redis")]
async fn start_cluster(config: &TtsServerConfig) {
    use fcp_cluster::{ClusterManager, ClusterMember};
    use std::time::{SystemTime, UNIX_EPOCH};

    let redis_config = fcp_storage_redis::RedisConfig {
        url: config.cluster.redis_url.clone(),
        ..Default::default()
    };
    let redis = match fcp_storage_redis::RedisStorage::new(redis_config).await {
        Ok(r) => r,
        Err(e) => {
            warn!(error = %e, "Failed to connect to Redis for cluster — running standalone");
            return;
        }
    };

    let member = ClusterMember {
        node_id: config.service.node_id.clone(),
        node_type: config.service.service_type.clone(),
        http_address: config
            .http
            .bind_address
            .parse()
            .unwrap_or_else(|_| "0.0.0.0:8093".parse().unwrap()),
        sip_address: "0.0.0.0:0".parse().unwrap(),
        sip_tls_address: None,
        capabilities: vec!["tts".to_string(), "synthesis".to_string()],
        active_calls: 0,
        started_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        public_url: None,
        capacity: None,
        draining: false,
        region: fcp_cluster::resolve_region(config.cluster.region.as_deref()),
        video_budget: Vec::new(),
        registering_trunks: Vec::new(),
        view: None,
    };

    let cm = ClusterManager::new(redis, member).with_intervals(
        config.cluster.heartbeat_interval_secs,
        config.cluster.heartbeat_ttl_secs,
    );
    cm.start();
    info!(
        node_id = %config.service.node_id,
        "Cluster membership registered"
    );
}

// ---------------------------------------------------------------------------
// Tracing
// ---------------------------------------------------------------------------

fn init_tracing(config: &TtsServerConfig) {
    // One setup for every binary, its lines shipped to Redis once the
    // service beacon starts (`docs/LOG_VIEWER.md`).
    if let Err(e) = fcp_cluster::logging::init(&config.logging.level, &config.logging.format, true)
    {
        eprintln!("logging: {e:#}");
    }
}

// ---------------------------------------------------------------------------
// Shutdown
// ---------------------------------------------------------------------------

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to listen for Ctrl+C");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to listen for SIGTERM")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => info!("Received Ctrl+C, shutting down"),
        _ = terminate => info!("Received SIGTERM, shutting down"),
    }
}

/// A comma-separated list from the environment, as a token list.
fn split_tokens(v: &str) -> Vec<String> {
    v.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider_entry(provider_type: &str) -> config::ProviderEntry {
        config::ProviderEntry {
            name: "primary".to_string(),
            provider_type: provider_type.to_string(),
            engine: Some("kokoro".to_string()),
            python_path: Some("python3".to_string()),
            api_key: Some("test-key".to_string()),
            model: Some("tts-1".to_string()),
            device: "cuda:0".to_string(),
            binary: None,
            voices_dir: None,
            voice: None,
            region: None,
            access_key_id: None,
            secret_access_key: None,
            endpoint: None,
        }
    }

    #[test]
    fn test_build_provider_config_rejects_missing_api_key() {
        let mut entry = provider_entry("openai");
        entry.api_key = Some("   ".to_string());

        let error = build_provider_config(&entry).unwrap_err();
        assert!(error.to_string().contains("requires a non-empty api_key"));
    }

    #[test]
    fn test_build_provider_config_passes_device_to_local_provider() {
        let provider = build_provider_config(&provider_entry("local")).unwrap();

        match provider {
            fcp_media::tts::TtsProviderConfig::Local { device, .. } => {
                assert_eq!(device.as_deref(), Some("cuda:0"));
            }
            other => panic!("unexpected provider config: {:?}", other),
        }
    }

    #[test]
    fn test_extract_bearer_token_accepts_common_forms() {
        assert_eq!(extract_bearer_token("Bearer token-1"), Some("token-1"));
        assert_eq!(extract_bearer_token("bearer token-2"), Some("token-2"));
        assert_eq!(extract_bearer_token("token-3"), None);
    }

    #[test]
    fn test_constant_time_compare_requires_exact_match() {
        assert!(fcp_core::secret::constant_time_eq("secret", "secret"));
        assert!(!fcp_core::secret::constant_time_eq("secret", "Secret"));
        assert!(!fcp_core::secret::constant_time_eq("secret", "secret-2"));
    }
}
