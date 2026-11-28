//! `fcp-backup`: the platform's database and files to restic repositories
//! off the box, as the admin sets them up (`docs/SCHEDULED_BACKUPS.md`).
//!
//! Every `[backup] poll_secs` it asks for each destination's scheduled run
//! that has come due, then claims and carries out every requested run (a
//! test, "Back up now", or the schedule's), one at a time, and says how
//! each went: on its row, and as `system.backup_completed` or
//! `system.backup_failed`.

mod api;
mod config;
mod files;
mod restic;
mod restore_system;
mod runner;

use anyhow::{Context, Result};
use clap::Parser;
use fcp_core::backup::{daily_due, BackupRun, BackupRunKind, BackupRunState, BackupStore};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tracing::{error, info, warn};

#[derive(Parser, Debug)]
#[command(name = "fcp-backup", version = env!("CARGO_PKG_VERSION"))]
struct Cli {
    /// The TOML configuration.
    #[arg(
        short = 'c',
        long = "config",
        value_name = "PATH",
        env = "FCP_BACKUP_CONFIG"
    )]
    config: String,
    /// Load and validate the configuration, print it with every secret
    /// masked, and exit.
    #[arg(long = "check-config")]
    check_config: bool,
    /// `[storage] database_url`, from the environment.
    #[arg(
        long = "database-url",
        env = "FCP_BACKUP_DATABASE_URL",
        hide_env_values = true
    )]
    database_url: Option<String>,
    /// `[events] auth_token`, from the environment.
    #[arg(
        long = "events-auth-token",
        env = "FCP_BACKUP_EVENTS_AUTH_TOKEN",
        hide_env_values = true
    )]
    events_auth_token: Option<String>,
    /// `[http] token`, the admin's bearer token, from the environment.
    #[arg(
        long = "api-token",
        env = "FCP_BACKUP_API_TOKEN",
        hide_env_values = true
    )]
    api_token: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut config =
        fcp_core::config::check_stage(config::load_config(&cli.config), cli.check_config)?;
    if let Some(v) = cli.database_url {
        config.storage.database_url = v;
    }
    if let Some(v) = cli.events_auth_token {
        config.events.auth_token = Some(v);
    }
    if let Some(v) = cli.api_token {
        config.http.token = Some(v);
    }
    fcp_core::config::check_stage(config::validate(&config), cli.check_config)?;
    if cli.check_config {
        println!(
            "{}",
            fcp_core::config::masked_toml(&config).map_err(anyhow::Error::msg)?
        );
        return Ok(());
    }
    fcp_cluster::logging::init(&config.logging.level, &config.logging.format, true)?;
    info!(version = env!("CARGO_PKG_VERSION"), node = %config.service.node_id, "fcp-backup starting");

    let storage = Arc::new(
        fcp_storage_postgres::PostgresStorage::new(fcp_storage_postgres::PostgresConfig::new(
            config.storage.database_url.clone(),
        ))
        .await
        .context("[storage] database_url")?,
    );
    tokio::fs::create_dir_all(&config.backup.work_dir)
        .await
        .with_context(|| format!("[backup] work_dir {}", config.backup.work_dir.display()))?;

    let beacon = config.cluster.redis_url.clone().map(|url| {
        fcp_cluster::ServiceBeacon::new(
            fcp_cluster::ServiceIdentity::new(&config.service.node_id, "backup")
                .http(&config.http.bind_address),
        )
        .start(fcp_storage_redis::RedisConfig {
            url,
            ..Default::default()
        })
    });

    let ctx = Arc::new(runner::Ctx {
        config: config.clone(),
        store: storage.clone() as Arc<dyn BackupStore>,
    });

    // /health and /ready, and the admin's API when there is a token.
    let bind = config.http.bind_address.clone();
    let mut app = axum::Router::new()
        .route("/health", axum::routing::get(|| async { "ok" }))
        .route("/ready", axum::routing::get(|| async { "ok" }));
    match config.http.token.clone().filter(|t| !t.is_empty()) {
        Some(token) => {
            app = app.merge(api::router(api::ApiState {
                ctx: Arc::clone(&ctx),
                token,
                downloads: Default::default(),
                restore: Default::default(),
            }));
        }
        None => {
            info!("fcp-backup: no [http] token, so the admin cannot list or download snapshots")
        }
    }
    tokio::spawn(async move {
        match tokio::net::TcpListener::bind(&bind).await {
            Ok(l) => {
                let _ = axum::serve(l, app).await;
            }
            Err(e) => error!(bind = %bind, error = %e, "fcp-backup could not bind its HTTP port"),
        }
    });
    let events = Events::new(&config);
    let mut tick = tokio::time::interval(Duration::from_secs(config.backup.poll_secs));
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .context("SIGTERM")?;
    loop {
        tokio::select! {
            _ = tick.tick() => {}
            _ = tokio::signal::ctrl_c() => break,
            _ = term.recv() => break,
        }
        if let Err(e) = schedule(&ctx).await {
            warn!(error = %e, "Backups: the schedule could not be read");
        }
        loop {
            let run = match ctx.store.claim_backup_run(&config.service.node_id).await {
                Ok(Some(run)) => run,
                Ok(None) => break,
                Err(e) => {
                    warn!(error = %e, "Backups: a run could not be claimed");
                    break;
                }
            };
            carry(&ctx, &events, run).await;
        }
    }
    if let Some(b) = beacon {
        b.shutdown().await;
    }
    info!("fcp-backup stopped");
    Ok(())
}

/// Ask for each enabled destination's scheduled run that has come due.
async fn schedule(ctx: &runner::Ctx) -> Result<()> {
    let now = SystemTime::now();
    for d in ctx.store.list_backup_destinations().await? {
        let Some(at) = d.schedule.daily_at.as_deref().filter(|_| d.enabled) else {
            continue;
        };
        let last = ctx.store.last_scheduled_run(&d.id).await?;
        if daily_due(at, last.map(|r| r.requested_at), now) == Some(true) {
            info!(destination = %d.name, "Backups: a scheduled run is due");
            ctx.store
                .request_backup_run(&BackupRun {
                    id: uuid::Uuid::new_v4(),
                    destination_id: d.id,
                    kind: BackupRunKind::Backup,
                    state: BackupRunState::Requested,
                    trigger: "schedule".into(),
                    requested_at: now,
                    started_at: None,
                    finished_at: None,
                    snapshots: Vec::new(),
                    bytes_added: 0,
                    message: None,
                    node: None,
                })
                .await?;
        }
    }
    Ok(())
}

async fn carry(ctx: &runner::Ctx, events: &Events, mut run: BackupRun) {
    let started = std::time::Instant::now();
    let dest = match ctx.store.get_backup_destination(&run.destination_id).await {
        Ok(Some(d)) => d,
        Ok(None) => return,
        Err(e) => {
            warn!(error = %e, "Backups: the destination could not be read");
            return;
        }
    };
    info!(destination = %dest.name, kind = run.kind.as_str(), trigger = %run.trigger, "Backups: a run starts");
    let outcome = runner::carry_out(ctx, &dest, &run).await;
    run.finished_at = Some(SystemTime::now());
    let event = match outcome {
        Ok(o) => {
            run.state = BackupRunState::Succeeded;
            run.snapshots = o.snapshots;
            run.bytes_added = o.bytes_added;
            run.message = Some(o.message);
            info!(destination = %dest.name, message = ?run.message, "Backups: the run succeeded");
            (run.kind == BackupRunKind::Backup).then(|| fcp_events::SystemEvent::BackupCompleted {
                destination_id: format!("bkd-{}", dest.id),
                destination: dest.name.clone(),
                run_id: format!("bkr-{}", run.id),
                snapshots: run.snapshots.clone(),
                bytes_added: run.bytes_added,
                duration_secs: started.elapsed().as_secs(),
                timestamp: SystemTime::now(),
            })
        }
        Err(e) => {
            run.state = BackupRunState::Failed;
            run.message = Some(format!("{e:#}"));
            warn!(destination = %dest.name, error = %format!("{e:#}"), "Backups: the run failed");
            Some(fcp_events::SystemEvent::BackupFailed {
                destination_id: format!("bkd-{}", dest.id),
                destination: dest.name.clone(),
                run_id: format!("bkr-{}", run.id),
                kind: run.kind.as_str().into(),
                error: format!("{e:#}"),
                timestamp: SystemTime::now(),
            })
        }
    };
    if let Err(e) = ctx.store.finish_backup_run(&run).await {
        warn!(error = %e, "Backups: a run's outcome could not be kept");
    }
    if let Some(event) = event {
        events
            .publish(fcp_events::PlatformEvent::System(event))
            .await;
    }
}

/// The events server, when there is one.
struct Events {
    url: Option<String>,
    token: Option<String>,
    node: String,
    client: reqwest::Client,
}

impl Events {
    fn new(config: &config::BackupConfig) -> Self {
        Self {
            // The events server's address; `/ingest` is added unless named.
            url: config.events.server_url.clone().map(|u| {
                let u = u.trim_end_matches('/');
                if u.ends_with("/ingest") {
                    u.to_string()
                } else {
                    format!("{u}/ingest")
                }
            }),
            token: config.events.auth_token.clone(),
            node: config.service.node_id.clone(),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap_or_default(),
        }
    }

    async fn publish(&self, event: fcp_events::PlatformEvent) {
        let Some(url) = &self.url else { return };
        let mut req = self
            .client
            .post(url)
            .header("X-Node-ID", &self.node)
            .json(&event);
        if let Some(t) = &self.token {
            req = req.bearer_auth(t);
        }
        if let Err(e) = req.send().await.and_then(|r| r.error_for_status()) {
            warn!(error = %e, "Backups: an event could not be sent");
        }
    }
}
