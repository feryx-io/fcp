//! fcp-tts — Dedicated shared Text-to-Speech service.
//!
//! Centralizes TTS synthesis so that call-manager, conference, IVR, and admin
//! services share a single Kokoro/cloud TTS backend instead of each spawning
//! their own subprocess.

mod cli;
mod config;
mod routes;
mod state;

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::{
    extract::Request,
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
    let mut config = config::load_config(cli.config.as_deref())?;
    if let Some(ref level) = cli.log_level {
        config.logging.level = level.clone();
    }
    if let Some(ref fmt) = cli.log_format {
        config.logging.format = fmt.clone();
    }
    if let Some(ref bind) = cli.bind {
        config.http.bind_address = bind.clone();
    }

    // 3. Tracing
    init_tracing(&config);

    info!(
        version = env!("CARGO_PKG_VERSION"),
        node_id = %config.service.node_id,
        bind = %config.http.bind_address,
        providers = config.tts.providers.len(),
        "fcp-tts starting"
    );

    // 4. Create TTS providers
    let mut services: HashMap<String, Arc<fcp_media::tts::TtsService>> = HashMap::new();

    for entry in &config.tts.providers {
        // Set GPU device env var for local providers (read by kokoro_tts_worker.py)
        if matches!(entry.provider_type.as_str(), "local" | "kokoro") {
            std::env::set_var("KOKORO_DEVICE", &entry.device);
        }
        let provider_config = build_provider_config(entry);
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

    if services.is_empty() {
        warn!("No TTS providers initialized — service will return errors for synthesis requests");
    }

    let app_state = Arc::new(AppState {
        default_provider: config.tts.default_provider.clone(),
        services,
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

    // 7. Bind and serve
    let bind_addr: std::net::SocketAddr = config
        .http
        .bind_address
        .parse()
        .context("Invalid HTTP bind address")?;

    info!(
        bind = %bind_addr,
        default_provider = %config.tts.default_provider,
        "fcp-tts ready"
    );

    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    info!("fcp-tts shutdown complete");
    Ok(())
}

// ---------------------------------------------------------------------------
// Auth middleware
// ---------------------------------------------------------------------------

async fn require_api_token(
    tokens: Vec<String>,
    req: Request,
    next: Next,
) -> Response {
    // Skip auth for /health
    if req.uri().path() == "/health" {
        return next.run(req).await;
    }

    // No tokens configured = auth disabled
    if tokens.is_empty() {
        return next.run(req).await;
    }

    let auth_header = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok());

    if let Some(header) = auth_header {
        let token = header.strip_prefix("Bearer ").unwrap_or(header);
        if tokens.iter().any(|t| t == token) {
            return next.run(req).await;
        }
    }

    (StatusCode::UNAUTHORIZED, "Invalid or missing authentication token").into_response()
}

// ---------------------------------------------------------------------------
// Provider config builder
// ---------------------------------------------------------------------------

fn build_provider_config(entry: &config::ProviderEntry) -> fcp_media::tts::TtsProviderConfig {
    match entry.provider_type.as_str() {
        "local" | "kokoro" => fcp_media::tts::TtsProviderConfig::Local {
            engine: entry
                .engine
                .clone()
                .unwrap_or_else(|| "kokoro".to_string()),
            executable_path: entry.python_path.as_ref().map(std::path::PathBuf::from),
        },
        "openai" => fcp_media::tts::TtsProviderConfig::OpenAi {
            api_key: entry.api_key.clone().unwrap_or_default(),
            model: entry
                .model
                .clone()
                .unwrap_or_else(|| "tts-1".to_string()),
        },
        "google_cloud" => fcp_media::tts::TtsProviderConfig::GoogleCloud {
            api_key: entry.api_key.clone().unwrap_or_default(),
        },
        other => {
            warn!(provider_type = %other, "Unknown provider type — will fail at runtime");
            // Return a local config that will fail gracefully
            fcp_media::tts::TtsProviderConfig::Local {
                engine: other.to_string(),
                executable_path: None,
            }
        }
    }
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
        capabilities: vec!["tts".to_string(), "synthesis".to_string()],
        active_calls: 0,
        started_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        version: env!("CARGO_PKG_VERSION").to_string(),
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
    use tracing_subscriber::{fmt, prelude::*, EnvFilter};

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(&config.logging.level));

    match config.logging.format.as_str() {
        "json" => {
            tracing_subscriber::registry()
                .with(filter)
                .with(fmt::layer().json())
                .init();
        }
        "compact" => {
            tracing_subscriber::registry()
                .with(filter)
                .with(fmt::layer().compact())
                .init();
        }
        _ => {
            tracing_subscriber::registry()
                .with(filter)
                .with(fmt::layer())
                .init();
        }
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
