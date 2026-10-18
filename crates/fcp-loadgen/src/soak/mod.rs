//! The soak (`docs/DEEP_TESTING.md` D6): the whole stack under steady
//! load for hours — calls between phones, two conference rooms that fill
//! and empty, the phones re-registering, an administrator and a user
//! through the API — while [`stack::StackProbe`] records what every
//! service and store holds, and [`growth::judge`] fails the run on
//! growth that does not level off.
//!
//! The load is [`crate::run`]'s, three runs at once; the soak adds the
//! seeding, the API traffic, the stack's view and the verdict.

pub mod admin;
pub mod growth;
pub mod stack;

use crate::budget::Budget;
use crate::profile::Profile;
use crate::report::Report;
use crate::run::RunOptions;
use crate::target::Target;
use admin::{Admin, ApiCounts, SeedPlan};
use anyhow::{Context, Result};
use growth::Verdict;
use serde::Serialize;
use stack::{StackProbe, StackSample};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::info;

/// The account every soak phone shares; the admin API's password policy
/// accepts it.
pub const PASSWORD: &str = "Soak-steady-2026!";
const TENANT: &str = "soak";
/// The phones that answer are `peer1…` on extensions from here.
const PEER_EXTENSION_BASE: u32 = 2001;
/// The user the API traffic signs in as; it places no calls.
const API_USER: &str = "soakapi";

/// A conference room and the callers that keep it busy.
#[derive(Debug, Clone)]
pub struct Room {
    pub number: String,
    pub callers: u32,
    /// How long each caller stays before hanging up and rejoining.
    pub call_secs: u64,
}

/// What a soak does.
pub struct SoakOptions {
    pub admin: Admin,
    /// The call manager's SIP address.
    pub sip: SocketAddr,
    pub realm: String,
    pub duration: Duration,
    /// Left out of the verdict: caches warm and pools fill.
    pub warmup: Duration,
    pub sample_interval: Duration,
    /// Callers `load1…`, each answered by `peer<n>`.
    pub callers: u32,
    pub cps: f64,
    pub call_secs: u64,
    pub rooms: Vec<Room>,
    /// One round of API traffic this often.
    pub api_every: Duration,
}

impl SoakOptions {
    /// The seeding the options need.
    fn plan(&self) -> SeedPlan {
        let mut users = Vec::new();
        for i in 1..=self.callers {
            users.push((format!("load{i}"), None));
            users.push((
                format!("peer{i}"),
                Some((PEER_EXTENSION_BASE + i - 1).to_string()),
            ));
        }
        let room_callers: u32 = self.rooms.iter().map(|r| r.callers).sum();
        for i in 1..=room_callers {
            users.push((format!("conf{i}"), None));
        }
        users.push((API_USER.to_string(), None));
        SeedPlan {
            tenant: TENANT.to_string(),
            realm: self.realm.clone(),
            password: PASSWORD.to_string(),
            users,
            rooms: self.rooms.iter().map(|r| r.number.clone()).collect(),
        }
    }

    /// The three loads: the calls, and each room's callers.
    fn loads(&self) -> Vec<(String, RunOptions)> {
        // Nothing refused, dropped or silent, over hours.
        let budget = Budget {
            max_refused_fraction: Some(0.0),
            max_dropped: Some(0),
            max_silent_legs: Some(0),
            ..Budget::default()
        };
        let target = |prefix: &str, first_user: u32, destination: &str| Target {
            sip: self.sip,
            realm: self.realm.clone(),
            user_prefix: prefix.to_string(),
            first_user,
            password: PASSWORD.to_string(),
            destination: destination.to_string(),
            metrics_url: None,
            pid: None,
        };
        let mut loads = Vec::new();
        if self.callers > 0 {
            loads.push((
                "calls".to_string(),
                RunOptions {
                    profile: Profile::Mixed,
                    target: target("load", 1, "{callee}"),
                    users: self.callers,
                    answer_with: Some("peer".to_string()),
                    answer_extension_base: Some(PEER_EXTENSION_BASE),
                    duration: self.duration,
                    call_secs: self.call_secs,
                    cps: self.cps,
                    // The phones refresh every minute; the mixed profile's
                    // storm re-registers them all every three quarters of it.
                    register_expiry: 120,
                    budget: budget.clone(),
                    ..RunOptions::default()
                },
            ));
        }
        let mut first = 1;
        for room in &self.rooms {
            loads.push((
                format!("room {}", room.number),
                RunOptions {
                    profile: Profile::Soak,
                    target: target("conf", first, &room.number),
                    users: room.callers,
                    answer_with: None,
                    duration: self.duration,
                    call_secs: room.call_secs,
                    register_expiry: 120,
                    budget: budget.clone(),
                    ..RunOptions::default()
                },
            ));
            first += room.callers;
        }
        loads
    }
}

/// What a soak found.
#[derive(Debug, Serialize)]
pub struct SoakReport {
    pub version: String,
    pub started_at: String,
    pub duration_secs: f64,
    pub warmup_secs: f64,
    /// Each load's own report, by name.
    pub loads: BTreeMap<String, Report>,
    /// The API traffic: per operation, per status.
    pub api: BTreeMap<String, BTreeMap<u16, u64>>,
    pub api_failures: u64,
    pub samples: Vec<StackSample>,
    /// Every series judged; empty when the run was too short to judge.
    pub verdicts: Vec<Verdict>,
}

impl SoakReport {
    /// Growth that did not level off, unexplained.
    pub fn leaks(&self) -> Vec<&Verdict> {
        self.verdicts.iter().filter(|v| v.fails()).collect()
    }

    /// Every load within its budget, the API answering, and nothing
    /// growing that should not — judged at all.
    pub fn passed(&self) -> bool {
        self.loads.values().all(Report::within_budget)
            && self.api_failures == 0
            && !self.verdicts.is_empty()
            && self.leaks().is_empty()
    }

    pub fn summary(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "soak: {:.0} min (warm-up {:.0} min), {} samples",
            self.duration_secs / 60.0,
            self.warmup_secs / 60.0,
            self.samples.len()
        );
        for (name, report) in &self.loads {
            let m = &report.measurements;
            let _ = writeln!(
                out,
                "  {name:<12} offered {} answered {} refused {} dropped {} silent legs {} registers {}  {}",
                m.offered,
                m.answered,
                m.refused,
                m.dropped,
                m.media.silent_legs,
                m.registers,
                if report.within_budget() { "ok" } else { "OVER BUDGET" }
            );
        }
        let calls: u64 = self.api.values().flat_map(|c| c.values()).sum();
        let _ = writeln!(
            out,
            "  api          {calls} requests, {} failed",
            self.api_failures
        );
        if self.verdicts.is_empty() {
            let _ = writeln!(out, "  too few samples after the warm-up to judge growth");
        }
        let mut grew: Vec<&Verdict> = self.verdicts.iter().filter(|v| v.grows).collect();
        grew.sort_by_key(|v| v.expected.is_some());
        for v in grew {
            let _ = writeln!(
                out,
                "  {} {:<48} {:>14.0} {:>14.0} {:>14.0} {:>14.0}{}",
                if v.fails() { "GROWS" } else { "grows" },
                v.series,
                v.windows[0],
                v.windows[1],
                v.windows[2],
                v.windows[3],
                v.expected
                    .map(|why| format!("  ({why})"))
                    .unwrap_or_default()
            );
        }
        let _ = writeln!(
            out,
            "  verdict: {}",
            if self.passed() { "passed" } else { "FAILED" }
        );
        out
    }
}

/// Seed, load, sample with `probe`, judge.
pub async fn run(opts: SoakOptions, mut probe: StackProbe) -> Result<SoakReport> {
    let started_at = chrono::Utc::now().to_rfc3339();
    let plan = opts.plan();
    let tenant = admin::seed(&opts.admin, &plan)
        .await
        .context("seeding the soak's tenant")?;
    let (_, api_user) = opts
        .admin
        .call(
            "GET",
            &format!("/api/v1/users/{API_USER}"),
            Some(&tenant),
            None,
            None,
        )
        .await?;
    let api_user_id = api_user["id"]
        .as_str()
        .context("the API user's id")?
        .to_string();
    info!(%tenant, users = plan.users.len(), rooms = plan.rooms.len(), "seeded");

    let began = Instant::now();
    let stop = Arc::new(AtomicBool::new(false));

    // The stack's view, from before the load starts.
    let interval = opts.sample_interval;
    let probe_stop = Arc::clone(&stop);
    let sampler = tokio::spawn(async move {
        let mut samples = Vec::new();
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        while !probe_stop.load(Ordering::Relaxed) {
            ticker.tick().await;
            let sample = probe.sample(began.elapsed().as_secs_f64()).await;
            if samples.len() % 20 == 0 {
                info!(
                    at_min = (sample.at_secs / 60.0).round(),
                    series = sample.series.len(),
                    "sampled"
                );
            }
            samples.push(sample);
        }
        samples
    });

    // The API traffic.
    let counts = Arc::new(ApiCounts::default());
    let api = {
        let stop = Arc::clone(&stop);
        let counts = Arc::clone(&counts);
        let admin = Admin {
            base: opts.admin.base.clone(),
            token: opts.admin.token.clone(),
        };
        let every = opts.api_every;
        let tenant = tenant.clone();
        tokio::spawn(async move {
            let mut round = 0;
            while !stop.load(Ordering::Relaxed) {
                admin::api_round(
                    &admin,
                    &tenant,
                    API_USER,
                    &api_user_id,
                    PASSWORD,
                    round,
                    &counts,
                )
                .await;
                round += 1;
                tokio::time::sleep(every).await;
            }
        })
    };

    // The calls and the rooms, until the duration is up.
    let runs: Vec<_> = opts
        .loads()
        .into_iter()
        .map(|(name, load)| tokio::spawn(async move { (name, crate::run(load).await) }))
        .collect();
    let mut loads = BTreeMap::new();
    for run in runs {
        let (name, report) = run.await.context("a load panicked")?;
        loads.insert(name, report?);
    }

    stop.store(true, Ordering::Relaxed);
    let _ = api.await;
    let samples = sampler.await.context("the sampler panicked")?;
    let verdicts = growth::judge(&samples, opts.warmup.as_secs_f64());
    Ok(SoakReport {
        version: env!("CARGO_PKG_VERSION").to_string(),
        started_at,
        duration_secs: began.elapsed().as_secs_f64(),
        warmup_secs: opts.warmup.as_secs_f64(),
        loads,
        api: counts.snapshot(),
        api_failures: counts.failures(),
        samples,
        verdicts,
    })
}
