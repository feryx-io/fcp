//! fcp-media-server — Standalone REST media processing server.
//!
//! Wraps forge-api's `ApiServer` with FCP-style TOML configuration,
//! CLI argument parsing, and structured tracing. Provides session
//! management, conferencing, WebRTC, AI integration, Prometheus
//! metrics, authentication, rate limiting, TLS, and graceful shutdown.
//!
//! This binary is pure HTTP/RTP — no SIP stack is included.

use anyhow::{Context, Result};
use clap::Parser;
use forge_api::ApiServer;
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

mod config;

use config::MediaServerConfig;

/// FCP Media Server - Standalone Media Processing Engine
#[derive(Parser, Debug)]
#[command(
    name = "fcp-media",
    version,
    about = "Standalone media processing server for the Ferrous Communications Platform",
    long_about = "A REST media server wrapping forge-api with session management,\n\
                  conferencing, WebRTC, AI integration, Prometheus metrics,\n\
                  authentication, rate limiting, TLS, and graceful shutdown."
)]
struct Cli {
    /// Path to configuration file
    #[arg(
        short = 'c',
        long = "config",
        value_name = "FILE",
        help = "Path to TOML configuration file",
        env = "FCP_MEDIA_CONFIG"
    )]
    config: Option<std::path::PathBuf>,

    /// Log level override
    #[arg(
        short = 'l',
        long = "log-level",
        value_name = "LEVEL",
        help = "Override log level (trace, debug, info, warn, error)",
        env = "FCP_MEDIA_LOG_LEVEL"
    )]
    log_level: Option<String>,

    /// Log format
    #[arg(
        long = "log-format",
        value_name = "FORMAT",
        help = "Log format (json, pretty, compact)",
        env = "FCP_MEDIA_LOG_FORMAT"
    )]
    log_format: Option<String>,

    /// HTTP bind address override
    #[arg(
        long = "http-bind",
        value_name = "ADDR",
        help = "Override HTTP bind address (e.g., 0.0.0.0:8080)",
        env = "FCP_MEDIA_HTTP_BIND"
    )]
    http_bind: Option<String>,

    /// Public IP address for RTP/SDP media
    #[arg(
        long = "public-ip",
        value_name = "IP",
        help = "Public IP for RTP (required in Docker or behind NAT)",
        env = "FCP_MEDIA_PUBLIC_IP"
    )]
    public_ip: Option<String>,
    /// Load and validate the configuration, print it with every secret
    /// masked, and exit: 0 when the node would start, 1 when a startup
    /// rule refuses it, 2 when it does not parse.
    #[arg(long = "check-config")]
    pub check_config: bool,
    /// `[storage] database_url` from the environment, for a secret store or a compose
    /// `.env` to inject; wins over the file (`docs/SHIP_AND_OPERATE.md`
    /// phase 2).
    #[arg(
        long = "database-url",
        value_name = "URL",
        env = "FCP_MEDIA_DATABASE_URL",
        hide_env_values = true
    )]
    database_url: Option<String>,

    /// `[auth] tokens`, comma-separated from the environment, for a secret store or a compose
    /// `.env` to inject; wins over the file (`docs/SHIP_AND_OPERATE.md`
    /// phase 2).
    #[arg(
        long = "auth-tokens",
        value_name = "TOKEN",
        env = "FCP_MEDIA_AUTH_TOKENS",
        hide_env_values = true
    )]
    auth_tokens: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    // rustls 0.23 needs the process to choose a crypto provider before
    // any TLS configuration is built (the HTTPS listener's is); with more
    // than one provider in the dependency graph it will not guess.
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    }

    // 1. Parse CLI arguments
    let cli = Cli::parse();

    // 2. Load configuration
    let mut config = fcp_core::config::check_stage(
        config::load_config(cli.config.as_deref()),
        cli.check_config,
    )?;
    // Secrets from the environment win over the file
    // (`docs/SHIP_AND_OPERATE.md` phase 2).
    if let Some(v) = &cli.database_url {
        config.storage.database_url = Some(v.clone());
    }
    if let Some(v) = &cli.auth_tokens {
        config.auth.tokens = split_tokens(v);
    }

    // Apply CLI overrides
    if let Some(ref level) = cli.log_level {
        config.logging.level = level.clone();
    }
    if let Some(ref format) = cli.log_format {
        config.logging.format = format.clone();
    }
    if let Some(ref bind) = cli.http_bind {
        config.http.bind_address = bind
            .parse()
            .map_err(|e| anyhow::anyhow!("Invalid HTTP bind address '{}': {}", bind, e))?;
    }
    if let Some(ref ip) = cli.public_ip {
        if !ip.is_empty() {
            config.media.public_ip = Some(ip.clone());
        }
    }

    // 3. Validate configuration
    fcp_core::config::check_stage(config::validate_config(&config), cli.check_config)?;

    // 4. Initialize tracing
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
        "fcp-media starting"
    );

    // 5. Initialize config store from PostgreSQL (if database_url is configured)
    let database: Option<Arc<fcp_storage_postgres::PostgresStorage>> = if let Some(ref db_url) =
        config.storage.database_url
    {
        match fcp_storage_postgres::PostgresStorage::new(fcp_storage_postgres::PostgresConfig {
            connection_string: db_url.clone(),
            read_replica_url: config.storage.read_replica_url.clone(),
            max_replica_lag: Duration::from_secs(config.storage.max_replica_lag_secs),
            max_connections: config.storage.max_connections,
            connection_timeout: Duration::from_secs(30),
            encryption: None,
        })
        .await
        {
            Ok(storage) => {
                info!("PostgreSQL config store initialized");
                Some(Arc::new(storage))
            }
            Err(e) => {
                warn!(error = %e, "Failed to initialize PostgreSQL config store, using TOML only");
                None
            }
        }
    } else {
        None
    };
    let config_store: Option<Arc<dyn fcp_core::config_store::ConfigStore>> = database
        .clone()
        .map(|s| s as Arc<dyn fcp_core::config_store::ConfigStore>);

    // 5b. Seed operational config sections into database (DB primary, TOML seed)
    // Bootstrap sections (service, http, logging, tls, storage) stay TOML-only.
    if let Some(ref cs) = config_store {
        let cs = cs.as_ref();
        let svc = "media";
        let tid = "_system";

        let sections: Vec<(&str, serde_json::Value)> = vec![
            (
                "media",
                serde_json::to_value(&config.media).unwrap_or_default(),
            ),
            (
                "recording",
                serde_json::to_value(&config.recording).unwrap_or_default(),
            ),
            (
                "auth",
                serde_json::to_value(&config.auth).unwrap_or_default(),
            ),
            ("ai", serde_json::to_value(&config.ai).unwrap_or_default()),
            (
                "cluster",
                serde_json::to_value(&config.cluster).unwrap_or_default(),
            ),
        ];

        for (section, toml_val) in sections {
            match fcp_core::reconcile_with_seed(cs, svc, section, tid, &toml_val).await {
                Ok(Some(entry)) => {
                    match section {
                        "media" => {
                            if let Ok(v) = serde_json::from_value(entry.value) {
                                config.media = v;
                            }
                        }
                        "recording" => {
                            if let Ok(v) = serde_json::from_value(entry.value) {
                                config.recording = v;
                            }
                        }
                        "auth" => {
                            // SECURITY: Auth config stays TOML-only.
                            // DB overrides could silently disable auth on public binds.
                            info!(
                                service = svc,
                                section, "Skipping DB override for auth (TOML-only for security)"
                            );
                        }
                        "ai" => {
                            if let Ok(v) = serde_json::from_value(entry.value) {
                                config.ai = v;
                            }
                        }
                        "cluster" => {
                            if let Ok(v) = serde_json::from_value(entry.value) {
                                config.cluster = v;
                            }
                        }
                        _ => {}
                    }
                    info!(
                        service = svc,
                        section,
                        version = entry.version,
                        "loaded config from database"
                    );
                }
                Ok(None) => {
                    match cs
                        .set_config(svc, section, tid, toml_val, Some("seed"))
                        .await
                    {
                        Ok(version) => info!(
                            service = svc,
                            section, version, "seeded config into database"
                        ),
                        Err(e) => {
                            warn!(service = svc, section, error = %e, "failed to seed config")
                        }
                    }
                }
                Err(e) => {
                    warn!(service = svc, section, error = %e, "failed to check DB config, using TOML");
                }
            }
        }
        info!("Operational config loading complete (DB primary, TOML seed)");

        // SECURITY: Re-validate config after DB overrides to catch any unsafe state
        config::validate_config(&config)?;
    }

    // 6. Convert to ApiServerConfig
    let api_config = config::to_api_server_config(&config);

    // 6. Create ApiServer
    let server = ApiServer::new(api_config).await;

    // Initialize cluster membership (if configured)
    #[cfg(feature = "storage-redis")]
    let _cluster_manager = if config.cluster.enabled {
        if let Some(ref url) = config.storage.redis_url {
            match fcp_storage_redis::RedisStorage::new(fcp_storage_redis::RedisConfig {
                url: url.clone(),
                ..Default::default()
            })
            .await
            {
                Ok(redis) => {
                    let member = fcp_cluster::ClusterMember {
                        node_id: config.service.node_id.clone(),
                        node_type: config.service.service_type.clone(),
                        http_address: config.http.bind_address,
                        sip_address: "0.0.0.0:0".parse().unwrap(),
                        sip_tls_address: None,
                        capabilities: vec![
                            "media".to_string(),
                            "rtp".to_string(),
                            "transcoding".to_string(),
                            "recording".to_string(),
                        ],
                        active_calls: 0,
                        started_at: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
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
                    let cm = fcp_cluster::ClusterManager::new(redis, member).with_intervals(
                        config.cluster.heartbeat_interval_secs,
                        config.cluster.heartbeat_ttl_secs,
                    );
                    let _handle = cm.start();
                    info!("Cluster membership started");
                    Some(cm)
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Failed to connect to Redis; cluster disabled");
                    None
                }
            }
        } else {
            tracing::warn!("cluster.enabled = true but redis_url is not set; cluster disabled");
            None
        }
    } else {
        None
    };

    // Say this process is there, member or not (`docs/SERVICES.md`).
    #[cfg(feature = "storage-redis")]
    let beacon = config.storage.redis_url.as_ref().map(|url| {
        let mut beacon = fcp_cluster::ServiceBeacon::new(
            fcp_cluster::ServiceIdentity::new(&config.service.node_id, "media")
                .region(fcp_cluster::resolve_region(
                    config.cluster.region.as_deref(),
                ))
                .member(config.cluster.enabled)
                .http(config.http.bind_address),
        );
        if let Some(database) = &database {
            beacon = beacon.check(fcp_cluster::Check::database(database.ping_check()));
        }
        beacon.start(fcp_storage_redis::RedisConfig {
            url: url.clone(),
            ..Default::default()
        })
    });

    info!(
        http = %config.http.bind_address,
        rtp_ports = format!("{}-{}", config.media.rtp_port_start, config.media.rtp_port_end),
        tls = config.tls.enabled,
        auth = !config.auth.disable_auth,
        "fcp-media ready — listening for API requests"
    );

    // 7b. Spawn config poll loop (checks DB every 10s for config changes)
    if let Some(cs) = config_store {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(10));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut last_version: i64 = 0;
            loop {
                interval.tick().await;
                match cs.get_latest_version("media", "_system").await {
                    Ok(v) if v > last_version => {
                        tracing::info!(
                            old_version = last_version,
                            new_version = v,
                            "config version changed in database"
                        );
                        last_version = v;
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::debug!(error = %e, "config poll: failed to check version");
                    }
                }
            }
        });
    }

    // 8. Serve with graceful shutdown on SIGINT/SIGTERM
    let shutdown_timeout = Duration::from_secs(config.service.shutdown_timeout_secs);
    let shutdown_signal = async move {
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("Failed to register SIGTERM handler");

        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("Received SIGINT (Ctrl+C), initiating graceful shutdown...");
            }
            _ = sigterm.recv() => {
                info!("Received SIGTERM, initiating graceful shutdown...");
            }
        }

        // Allow in-flight requests to complete within the timeout
        if !shutdown_timeout.is_zero() {
            info!(
                timeout_secs = shutdown_timeout.as_secs(),
                "Draining connections..."
            );
        }
    };

    server
        .serve_with_shutdown(shutdown_signal)
        .await
        .context("API server error")?;

    // Shutdown cluster manager
    #[cfg(feature = "storage-redis")]
    if let Some(ref cm) = _cluster_manager {
        cm.shutdown().await;
    }
    #[cfg(feature = "storage-redis")]
    if let Some(beacon) = &beacon {
        beacon.shutdown().await;
    }

    info!("Shutdown complete");
    Ok(())
}

fn init_tracing(config: &MediaServerConfig) {
    use tracing_subscriber::EnvFilter;

    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&config.logging.level));

    match config.logging.format.as_str() {
        "json" => {
            tracing_subscriber::fmt()
                .with_target(false)
                .with_level(true)
                .json()
                .with_env_filter(env_filter)
                .init();
        }
        "compact" => {
            tracing_subscriber::fmt()
                .with_target(false)
                .with_level(true)
                .compact()
                .with_env_filter(env_filter)
                .init();
        }
        _ => {
            tracing_subscriber::fmt()
                .with_target(false)
                .with_level(true)
                .with_env_filter(env_filter)
                .init();
        }
    }
}

/// A comma-separated list from the environment, as a token list.
fn split_tokens(v: &str) -> Vec<String> {
    v.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}
