//! `fcp-loadgen` — put a node under load and write down what it did
//! (`docs/CAPACITY_AND_SURVIVABILITY.md` §8).
//!
//! ```text
//! fcp-loadgen --profile call-ramp --sip 10.0.0.7:5060 --realm fcp.local \
//!     --users 50 --cps 5 --cps-step 5 --cps-max 60 \
//!     --metrics-url http://10.0.0.7:8084/metrics --pid 4213 \
//!     --json ramp.json
//! ```
//!
//! The accounts must exist already: a run does not write to the
//! database, so what it measures is a node as deployed rather than a node
//! the harness has reached into.

use anyhow::{Context, Result};
use clap::Parser;
use fcp_loadgen::budget::Budget;
use fcp_loadgen::profile::Profile;
use fcp_loadgen::run::RunOptions;
use fcp_loadgen::target::Target;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Parser, Debug)]
#[command(
    name = "fcp-loadgen",
    version,
    about = "Load harness for FCP: many scripted SIP phones against a running node",
    long_about = "Drives fcp-testkit's scripted phones — real SIP, real RTP — against a \
                  running node, and reports what the callers saw beside what the node \
                  said about itself. The accounts it uses must already exist."
)]
struct Cli {
    /// What to do: register-storm, call-ramp, soak or mixed.
    #[arg(long, default_value = "soak")]
    profile: Profile,

    /// The node's SIP address.
    #[arg(long)]
    sip: SocketAddr,

    /// The SIP realm the accounts live in.
    #[arg(long, default_value = "fcp.local")]
    realm: String,

    /// Callers are `{prefix}{first_user + i}`.
    #[arg(long, default_value = "load")]
    user_prefix: String,

    /// The number the first caller's name ends with.
    #[arg(long, default_value_t = 1)]
    first_user: u32,

    /// The password every account shares.
    #[arg(long, default_value = "load")]
    password: String,

    /// How many callers to run.
    #[arg(long, default_value_t = 10)]
    users: u32,

    /// Answer the calls with phones named `{prefix}{first_user + i}`,
    /// registered and auto-answering. Without it every call goes to
    /// `--destination` as given.
    #[arg(long, default_value = "peer")]
    answer_with: String,

    /// The extension the first answering phone is reached at; the `i`th
    /// is that plus `i`. Without it `{callee}` is the phone's account
    /// name, which suits a deployment that routes by AOR.
    #[arg(long)]
    answer_extension_base: Option<u32>,

    /// Send the calls somewhere else — a room, an IVR, a number on a
    /// trunk — instead of to an answering phone. `{n}` is the caller's
    /// number, `{i}` its index and `{callee}` its answering phone.
    #[arg(long, default_value = "{callee}")]
    destination: String,

    /// How long the run lasts, in seconds.
    #[arg(long, default_value_t = 60)]
    seconds: u64,

    /// How long each answered call is held, in seconds.
    #[arg(long, default_value_t = 30)]
    call_seconds: u64,

    /// Calls offered per second: the rate for soak and mixed, the first
    /// rung for a ramp.
    #[arg(long, default_value_t = 1.0)]
    cps: f64,

    /// What each rung of a ramp adds, and how far the ladder goes.
    #[arg(long, default_value_t = 1.0)]
    cps_step: f64,
    #[arg(long, default_value_t = 50.0)]
    cps_max: f64,

    /// How long each rung is held, in seconds.
    #[arg(long, default_value_t = 20)]
    rung_seconds: u64,

    /// A rung fails past this fraction of refusals …
    #[arg(long, default_value_t = 0.01)]
    stop_refused_fraction: f64,

    /// … or past this p99 of INVITE → 200, in milliseconds.
    #[arg(long, default_value_t = 2000.0)]
    stop_p99_ms: f64,

    /// The registration lifetime a storm asks for, in seconds; devices
    /// re-register a quarter early, as real ones do.
    #[arg(long, default_value_t = 60)]
    register_expiry: u32,

    /// Calls in flight at once.
    #[arg(long, default_value_t = 500)]
    max_in_flight: usize,

    /// The node's Prometheus endpoint, sampled while the run goes on.
    #[arg(long)]
    metrics_url: Option<String>,

    /// The node's process id, for its resident memory and CPU. Only
    /// useful when the node runs on this machine.
    #[arg(long)]
    pid: Option<u32>,

    /// How often the node is sampled, in seconds.
    #[arg(long, default_value_t = 5)]
    sample_seconds: u64,

    /// Write the full report here as JSON.
    #[arg(long)]
    json: Option<PathBuf>,

    // ── Budgets: a breached one exits non-zero ──────────────────────
    #[arg(long)]
    max_p99_invite_ms: Option<f64>,
    #[arg(long)]
    min_admitted_cps: Option<f64>,
    #[arg(long)]
    max_rtp_loss: Option<f64>,
    #[arg(long)]
    max_refused_fraction: Option<f64>,
    #[arg(long)]
    max_dropped: Option<u64>,
    #[arg(long)]
    max_silent_legs: Option<u64>,
    #[arg(long)]
    max_p99_register_ms: Option<f64>,
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

    let options = RunOptions {
        profile: cli.profile,
        target: Target {
            sip: cli.sip,
            realm: cli.realm,
            user_prefix: cli.user_prefix,
            first_user: cli.first_user,
            password: cli.password,
            destination: cli.destination,
            metrics_url: cli.metrics_url,
            pid: cli.pid,
        },
        users: cli.users,
        answer_with: (!cli.answer_with.is_empty()).then_some(cli.answer_with),
        answer_extension_base: cli.answer_extension_base,
        duration: Duration::from_secs(cli.seconds),
        call_secs: cli.call_seconds,
        cps: cli.cps,
        cps_step: cli.cps_step,
        cps_max: cli.cps_max,
        rung_secs: cli.rung_seconds,
        stop_refused_fraction: cli.stop_refused_fraction,
        stop_p99_ms: cli.stop_p99_ms,
        register_expiry: cli.register_expiry,
        max_in_flight: cli.max_in_flight,
        sample_interval: Duration::from_secs(cli.sample_seconds.max(1)),
        tone_hz: 440.0,
        budget: Budget {
            max_p99_invite_ms: cli.max_p99_invite_ms,
            min_admitted_cps: cli.min_admitted_cps,
            max_rtp_loss: cli.max_rtp_loss,
            max_refused_fraction: cli.max_refused_fraction,
            max_dropped: cli.max_dropped,
            max_silent_legs: cli.max_silent_legs,
            max_p99_register_ms: cli.max_p99_register_ms,
        },
    };

    let report = fcp_loadgen::run(options).await?;
    print!("{}", report.summary());
    if let Some(path) = &cli.json {
        std::fs::write(path, serde_json::to_string_pretty(&report)?)
            .with_context(|| format!("could not write {}", path.display()))?;
        println!("  report   {}", path.display());
    }
    if !report.within_budget() {
        // A breached budget is the run's verdict, not an error in it.
        std::process::exit(1);
    }
    Ok(())
}
