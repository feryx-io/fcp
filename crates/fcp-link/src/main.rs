//! fcp-link: a site's link to its cloud (`docs/HYBRID_CONTROL_PLANE.md`
//! §3, phase 1). At a site — a tenant's own stack on the tenant's own
//! network — this process enrolls once, with the one-time token a tenant
//! administrator minted in the cloud, and holds the certificate and the
//! secret that enrollment gave in its state directory. From then on it
//! heartbeats what it finds at the site: its own release, and every node
//! of the site's cluster when it has the site's Redis. With the site's
//! Postgres (phase 3) it keeps that a copy of the tenant's configuration:
//! a snapshot from the cloud, then every change as the cloud journals it,
//! applied through the same storage crate the cloud writes with. Later
//! phases carry records up over the same link. It opens no inbound port
//! but its own health check, which also takes the site's pushes
//! (`POST /push`, `docs/PUSH.md` §2.4); every connection is to the cloud.

mod cli;
mod config;
mod events;
mod firmware;
mod link;
mod logs;
mod requests;
mod ship;
mod sync;

use anyhow::{Context, Result};
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use clap::Parser;
use std::sync::Arc;
use tokio::signal;
use tracing::{error, info, warn};

use crate::cli::Cli;
use crate::config::{load_config, LinkFileConfig};
use crate::link::{Link, LinkStatus};

#[derive(Clone)]
struct AppState {
    link: Arc<Link>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut config =
        fcp_core::config::check_stage(load_config(cli.config.as_deref()), cli.check_config)?;
    if let Some(v) = &cli.enrollment_token {
        config.link.enrollment_token = Some(v.clone());
    }
    if let Some(v) = &cli.cloud_url {
        config.link.cloud_url = v.clone();
    }
    if let Some(v) = &cli.link_url {
        config.link.link_url = v.clone();
    }
    if let Some(v) = &cli.site_database_url {
        config.site.database_url = Some(v.clone());
    }
    if let Some(v) = &cli.push_token {
        config.site.push_token = Some(v.clone());
    }
    if let Some(v) = &cli.log_level {
        config.logging.level = v.clone();
    }
    if let Some(v) = &cli.log_format {
        config.logging.format = v.clone();
    }
    fcp_core::config::check_stage(config::validate_config(&config), cli.check_config)?;
    if cli.check_config {
        let masked = mask(&config);
        println!(
            "{}",
            fcp_core::config::masked_toml(&masked).map_err(anyhow::Error::msg)?
        );
        return Ok(());
    }
    init_tracing(&config);
    info!(
        version = env!("CARGO_PKG_VERSION"),
        service = "fcp-link",
        "Starting FCP Link"
    );

    let link = Arc::new(Link::new(config.clone()).await?);
    let state = AppState {
        link: Arc::clone(&link),
    };
    let app = Router::new()
        .route("/health", get(health))
        .route("/status", get(status))
        .route("/push", post(push))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(config.health.bind_address)
        .await
        .with_context(|| format!("binding {}", config.health.bind_address))?;
    info!(bind = %config.health.bind_address, "Health check listening");
    let health_task = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            error!(error = %e, "The health listener stopped");
        }
    });

    // Say this process is there, to the site's Redis (`docs/SERVICES.md`):
    // the site's admin lists it, and its link carries it up.
    let beacon = config.site.redis_url.as_ref().map(|url| {
        let watched = Arc::clone(&link);
        let beacon = fcp_cluster::ServiceBeacon::new(
            fcp_cluster::ServiceIdentity::new(&config.service.node_id, "link")
                .http(config.health.bind_address),
        )
        // The cloud: not required, a site goes on alone.
        .check(fcp_cluster::Check::new("cloud", false, move || {
            let status = watched.status();
            async move {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let window = status.heartbeat_interval_secs.max(1) * 3;
                match status.last_heartbeat_ok_at {
                    Some(at) if now.saturating_sub(at) <= window => Ok(()),
                    _ if !status.enrolled => Err("not enrolled".to_string()),
                    _ => Err(status
                        .last_heartbeat_error
                        .unwrap_or_else(|| "no heartbeat answered".to_string())),
                }
            }
        }));
        beacon.start(fcp_storage_redis::RedisConfig {
            url: url.clone(),
            ..Default::default()
        })
    });

    let run = Arc::clone(&link);
    let run_task = tokio::spawn(async move { run.run().await });

    shutdown_signal().await;
    info!("Shutdown signal received");
    run_task.abort();
    health_task.abort();
    if let Some(beacon) = &beacon {
        beacon.shutdown().await;
    }
    info!("Shutdown complete");
    Ok(())
}

async fn health(State(state): State<AppState>) -> Json<serde_json::Value> {
    let status = state.link.status();
    Json(serde_json::json!({
        "status": "ok",
        "enrolled": status.enrolled,
        "link_up": status.last_heartbeat_ok_at.is_some(),
    }))
}

async fn status(State(state): State<AppState>) -> Json<LinkStatus> {
    Json(state.link.status())
}

/// POST /push — a push from one of the site's call managers, which holds
/// no keys: sent up the link for the cloud to send, and what came of it
/// passed back (`docs/PUSH.md` §2.4). Behind `[site] push_token`.
async fn push(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(push): Json<fcp_core::push::RelayedPush>,
) -> Response {
    let refuse = |status: StatusCode, error: &str| {
        (status, Json(serde_json::json!({ "error": error }))).into_response()
    };
    let Some(token) = state.link.push_token() else {
        return refuse(
            StatusCode::NOT_FOUND,
            "this link relays no pushes: set [site] push_token",
        );
    };
    let given = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    if given != Some(token) {
        return refuse(StatusCode::UNAUTHORIZED, "the push token is wrong");
    }
    match state.link.push(&push).await {
        Ok((status, body)) => (
            StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY),
            Json(body),
        )
            .into_response(),
        Err(e) => {
            warn!(error = %e, "A push could not be sent up the link");
            refuse(StatusCode::SERVICE_UNAVAILABLE, &format!("{e:#}"))
        }
    }
}

/// The configuration as `--check-config` shows it: the token masked.
fn mask(config: &LinkFileConfig) -> LinkFileConfig {
    let mut masked = config.clone();
    if masked.site.push_token.is_some() {
        masked.site.push_token = Some("***".to_string());
    }
    if masked.link.enrollment_token.is_some() {
        masked.link.enrollment_token = Some("***".to_string());
    }
    masked
}

fn init_tracing(config: &LinkFileConfig) {
    // One setup for every binary, its lines shipped to Redis once the
    // service beacon starts (`docs/LOG_VIEWER.md`).
    if let Err(e) = fcp_cluster::logging::init(&config.logging.level, &config.logging.format, true)
    {
        eprintln!("logging: {e:#}");
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(e) = signal::ctrl_c().await {
            warn!(error = %e, "Failed to listen for Ctrl-C");
        }
    };
    #[cfg(unix)]
    let terminate = async {
        match signal::unix::signal(signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(e) => warn!(error = %e, "Failed to listen for SIGTERM"),
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
}
