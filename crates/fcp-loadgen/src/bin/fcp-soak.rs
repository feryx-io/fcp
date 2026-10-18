//! `fcp-soak` — the compose stack under steady load for hours, and
//! whether anything it holds kept growing (`docs/DEEP_TESTING.md` D6).
//!
//! ```text
//! fcp-soak --compose-dir deploy/docker-compose --minutes 240 --report soak.json
//! ```
//!
//! Against a stack the compose project runs, every `fcp-*` container is
//! sampled with `docker exec`; with `--pid name=pid` the named processes
//! of this user's are sampled instead. The admin token and the Postgres
//! password come from the project's `.env`. The run exits non-zero when
//! a load went over its budget, the API failed, or something grew that
//! should not have.

use anyhow::{Context, Result};
use clap::Parser;
use fcp_loadgen::soak::stack::{self, MetricsSource, Process, StackProbe};
use fcp_loadgen::soak::{self, admin::Admin, Room, SoakOptions};
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Parser, Debug)]
#[command(
    name = "fcp-soak",
    version,
    about = "The compose stack under steady load for hours, and whether anything kept growing"
)]
struct Cli {
    /// The compose project: its `.env` gives the tokens, and its running
    /// `fcp-*` containers are sampled.
    #[arg(long, default_value = "deploy/docker-compose")]
    compose_dir: PathBuf,

    /// How long the load runs.
    #[arg(long, default_value_t = 240)]
    minutes: u64,

    /// Left out of the verdict; by default a fifth of the run, at most
    /// thirty minutes.
    #[arg(long)]
    warmup_minutes: Option<u64>,

    /// How often the stack is sampled.
    #[arg(long, default_value_t = 30)]
    sample_seconds: u64,

    /// The call manager's SIP address.
    #[arg(long, default_value = "127.0.0.1:5060")]
    sip: SocketAddr,

    #[arg(long, default_value = "fcp.local")]
    realm: String,

    /// The admin API.
    #[arg(long, default_value = "http://127.0.0.1:8080")]
    admin_url: String,

    /// Postgres, by default the compose project's on the host.
    #[arg(long)]
    database_url: Option<String>,

    /// Redis, by default the compose project's on the host.
    #[arg(long, default_value = "redis://127.0.0.1:6379")]
    redis_url: String,

    /// Sample this process of this user's as the named service instead of
    /// the compose project's containers: `fcp-admin=4213`. Repeatable.
    #[arg(long = "pid", value_parser = parse_pair)]
    pids: Vec<(String, String)>,

    /// The call manager's `/metrics`, whose gauges (the calls up) are
    /// recorded beside the samples, with the project's
    /// `FCP_CALL_MANAGER_API_TOKEN`.
    #[arg(long, default_value = "http://127.0.0.1:8084/metrics")]
    call_manager_metrics: String,

    /// Callers placing calls, each answered by its own phone.
    #[arg(long, default_value_t = 20)]
    callers: u32,

    /// Calls offered per second.
    #[arg(long, default_value_t = 0.5)]
    cps: f64,

    /// How long each call is held.
    #[arg(long, default_value_t = 30)]
    call_seconds: u64,

    /// A round of API traffic this often.
    #[arg(long, default_value_t = 5)]
    api_seconds: u64,

    /// Write the full report, every sample included, here as JSON.
    #[arg(long)]
    report: Option<PathBuf>,
}

fn parse_pair(s: &str) -> Result<(String, String), String> {
    s.split_once('=')
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .ok_or_else(|| format!("{s:?} is not name=value"))
}

/// `KEY=value` lines of a compose project's `.env`.
fn dotenv(dir: &Path) -> Result<BTreeMap<String, String>> {
    let path = dir.join(".env");
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .compact()
        .init();

    let env = dotenv(&cli.compose_dir)?;
    let get = |key: &str| {
        env.get(key)
            .cloned()
            .with_context(|| format!("{key} is not in the project's .env"))
    };
    let database_url = match cli.database_url {
        Some(url) => url,
        None => format!(
            "postgres://fcp:{}@127.0.0.1:5432/fcp",
            get("POSTGRES_PASSWORD")?
        ),
    };
    let processes = if cli.pids.is_empty() {
        stack::compose_processes(&cli.compose_dir).await?
    } else {
        cli.pids
            .iter()
            .map(|(service, pid)| {
                Ok(Process::Local {
                    service: service.clone(),
                    pid: pid.parse().with_context(|| format!("pid {pid}"))?,
                })
            })
            .collect::<Result<_>>()?
    };
    anyhow::ensure!(!processes.is_empty(), "no FCP services to sample");
    let metrics = vec![MetricsSource {
        service: "fcp-call-manager".into(),
        url: cli.call_manager_metrics,
        token: env.get("FCP_CALL_MANAGER_API_TOKEN").cloned(),
    }];
    let probe = StackProbe::new(
        processes,
        Some(&database_url),
        Some(&cli.redis_url),
        metrics,
    )
    .await;

    let duration = Duration::from_secs(cli.minutes * 60);
    let warmup = match cli.warmup_minutes {
        Some(m) => Duration::from_secs(m * 60),
        None => (duration / 5).min(Duration::from_secs(30 * 60)),
    };
    let options = SoakOptions {
        admin: Admin {
            base: cli.admin_url.trim_end_matches('/').to_string(),
            token: get("FCP_ADMIN_AUTH_TOKEN")?,
        },
        sip: cli.sip,
        realm: cli.realm,
        duration,
        warmup,
        sample_interval: Duration::from_secs(cli.sample_seconds.max(1)),
        callers: cli.callers,
        cps: cli.cps,
        call_secs: cli.call_seconds,
        rooms: default_rooms(),
        api_every: Duration::from_secs(cli.api_seconds.max(1)),
    };

    let report = soak::run(options, probe).await?;
    print!("{}", report.summary());
    if let Some(path) = &cli.report {
        std::fs::write(path, serde_json::to_string_pretty(&report)?)
            .with_context(|| format!("could not write {}", path.display()))?;
        println!("  report   {}", path.display());
    }
    if !report.passed() {
        std::process::exit(1);
    }
    Ok(())
}

/// A room that stays busy, and one whose two callers leave together so
/// it empties and is made again on every round.
fn default_rooms() -> Vec<Room> {
    vec![
        Room {
            number: "8801".into(),
            callers: 3,
            call_secs: 45,
        },
        Room {
            number: "8802".into(),
            callers: 2,
            call_secs: 20,
        },
    ]
}
