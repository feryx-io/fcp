//! Driving a run: the phones, the pacing, and what each profile does with
//! them (`docs/CAPACITY_AND_SURVIVABILITY.md` §8).
//!
//! Every profile is the same few moves in a different order — register a
//! set of phones, place calls at a rate, hold them with a tone, measure
//! what came back — so they share one call task and one pacer rather
//! than four copies.

use crate::budget::Budget;
use crate::profile::Profile;
use crate::report::{Counters, LegMedia, Measurements, Report, Rung};
use crate::sampler::Sampler;
use crate::target::Target;
use anyhow::{Context, Result};
use fcp_testkit::phone::CallPhase;
use fcp_testkit::Phone;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

/// What a run was told to do.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RunOptions {
    pub profile: Profile,
    #[serde(skip)]
    pub target: Target,
    /// Phones that place calls, or register in a storm.
    pub users: u32,
    /// Phones that answer them, named with this prefix; `None` sends every
    /// call to the destination as given (a room, an IVR, a trunk).
    pub answer_with: Option<String>,
    /// The extension the `i`th answering phone is reached at: what
    /// `{callee}` in a destination becomes. Without it `{callee}` is the
    /// phone's account name, for a deployment that routes by AOR rather
    /// than by extension.
    pub answer_extension_base: Option<u32>,
    /// How long the run lasts. A ramp climbs within it.
    pub duration: Duration,
    /// How long each answered call is held.
    pub call_secs: u64,
    /// Calls offered per second: the rate for `soak` and `mixed`, the
    /// first rung for `call-ramp`.
    pub cps: f64,
    /// What each rung adds, and how far the ladder goes.
    pub cps_step: f64,
    pub cps_max: f64,
    /// How long a rung is held before it is judged.
    pub rung_secs: u64,
    /// A rung fails when this fraction of calls is refused …
    pub stop_refused_fraction: f64,
    /// … or when the p99 of INVITE → `200 OK` passes this.
    pub stop_p99_ms: f64,
    /// A storm's registration lifetime, and so how often it re-registers.
    pub register_expiry: u32,
    /// Calls in flight at once, so a run that meets a slow node queues
    /// rather than opening sockets until it falls over.
    pub max_in_flight: usize,
    pub sample_interval: Duration,
    pub tone_hz: f32,
    pub budget: Budget,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            profile: Profile::Soak,
            target: Target {
                sip: "127.0.0.1:5060".parse().expect("a loopback address"),
                realm: "fcp.local".to_string(),
                user_prefix: "load".to_string(),
                first_user: 1,
                password: "load".to_string(),
                destination: "{callee}".to_string(),
                metrics_url: None,
                pid: None,
            },
            users: 10,
            answer_with: Some("peer".to_string()),
            answer_extension_base: None,
            duration: Duration::from_secs(60),
            call_secs: 30,
            cps: 1.0,
            cps_step: 1.0,
            cps_max: 50.0,
            rung_secs: 20,
            stop_refused_fraction: 0.01,
            stop_p99_ms: 2000.0,
            register_expiry: 60,
            max_in_flight: 500,
            sample_interval: Duration::from_secs(5),
            tone_hz: 440.0,
            budget: Budget::default(),
        }
    }
}

/// The phones a run drives: those that call, and those that answer.
struct Phones {
    callers: Vec<Arc<Phone>>,
    callees: Vec<Arc<Phone>>,
}

impl Phones {
    /// Start and register every phone the run needs. A phone that cannot
    /// register is reported and dropped rather than ending the run: a
    /// storm's whole point is that some of them fail.
    async fn start(opts: &RunOptions, counters: &Counters) -> Result<Self> {
        let mut callers = Vec::new();
        for i in 0..opts.users {
            let phone = Arc::new(opts.target.phone(i).await?);
            callers.push(phone);
        }
        let mut callees = Vec::new();
        if let Some(prefix) = &opts.answer_with {
            for i in 0..opts.users {
                let name = format!("{prefix}{}", opts.target.first_user + i);
                let phone = Phone::new(
                    &name,
                    &opts.target.password,
                    &opts.target.realm,
                    opts.target.sip,
                )
                .await
                .with_context(|| format!("phone {name} could not start"))?;
                callees.push(Arc::new(phone));
            }
        }
        let phones = Self { callers, callees };
        phones.register_all(opts, counters).await;
        Ok(phones)
    }

    /// Register every phone at once — which is the register storm, and
    /// the first thing every other profile needs.
    async fn register_all(&self, opts: &RunOptions, counters: &Counters) {
        let mut tasks = Vec::new();
        for phone in self.callers.iter().chain(self.callees.iter()) {
            let phone = Arc::clone(phone);
            let expiry = opts.register_expiry;
            tasks.push(tokio::spawn(async move { register(&phone, expiry).await }));
        }
        for task in tasks {
            match task.await {
                Ok((code, took)) => {
                    counters.registered(code);
                    counters.register.record(took);
                }
                Err(e) => warn!(error = %e, "a registration task failed"),
            }
        }
    }

    /// The account name of the phone answering caller `i`.
    fn callee_name(opts: &RunOptions, i: u32) -> Option<String> {
        opts.answer_with
            .as_ref()
            .map(|prefix| format!("{prefix}{}", opts.target.first_user + i))
    }

    /// What caller `i` dials to reach it: its extension where the
    /// deployment has one, its account name otherwise.
    fn callee_target(opts: &RunOptions, i: u32) -> Option<String> {
        opts.answer_with.as_ref()?;
        match opts.answer_extension_base {
            Some(base) => Some((base + i).to_string()),
            None => Self::callee_name(opts, i),
        }
    }
}

/// Answer the calls that arrive at one phone until the run is over, and
/// play a tone back on each.
///
/// The far end has to speak: a caller measuring loss and jitter on a leg
/// nobody is sending to would measure silence and call it total loss —
/// which is what the harness's own first run reported.
async fn answer_calls(
    phone: Arc<Phone>,
    opts: Arc<RunOptions>,
    counters: Arc<Counters>,
    until: Instant,
    stop: Arc<AtomicBool>,
) {
    let mut answering = Vec::new();
    while Instant::now() < until && !stop.load(Ordering::Relaxed) {
        let Some(incoming) = phone.incoming(Duration::from_millis(250)).await else {
            continue;
        };
        let opts = Arc::clone(&opts);
        let counters = Arc::clone(&counters);
        answering.push(tokio::spawn(async move {
            let call = match incoming.answer().await {
                Ok(call) => call,
                Err(e) => {
                    debug!(error = %e, "could not answer");
                    return;
                }
            };
            // A little longer than the caller holds it, so the tone does
            // not stop before the caller stops listening.
            call.media()
                .play_tone(opts.tone_hz, (opts.call_secs + 2) * 1000);
            call.ended(Duration::from_secs(opts.call_secs + 10)).await;
            let stats = call.media().stream_stats();
            counters.media(LegMedia {
                received: stats.received,
                sent: call.media().packets_sent(),
                lost: stats.lost,
                out_of_order: stats.out_of_order,
                duplicates: stats.duplicates,
                jitter_ms: stats.jitter_ms,
            });
        }));
    }
    for task in answering {
        let _ = task.await;
    }
}

/// One REGISTER, timed. A failure is a status code like any other, so a
/// storm reports what the registrar said rather than only that it said
/// something.
async fn register(phone: &Phone, expiry: u32) -> (u16, Duration) {
    let began = Instant::now();
    let code = phone.register_for(expiry).await.unwrap_or_else(|e| {
        debug!(error = %e, "REGISTER failed");
        0
    });
    (code, began.elapsed())
}

/// Place one call, hold it, and measure what came back.
async fn one_call(
    phone: Arc<Phone>,
    destination: String,
    opts: Arc<RunOptions>,
    counters: Arc<Counters>,
) {
    counters.offered();
    let call = match phone.call(&destination).await {
        Ok(call) => call,
        Err(e) => {
            debug!(error = %e, "INVITE could not be sent");
            counters.outcome(0);
            return;
        }
    };
    let answer_within = Duration::from_secs(20);
    let answered = call.answered(answer_within).await.is_ok();
    let timings = call.timings();
    if let Some((code, at)) = timings.first_provisional {
        counters.invite_to_provisional.record(at);
        let _ = code;
    }
    if let Some(at) = timings.ringing {
        counters.invite_to_ringing.record(at);
    }
    match timings.final_response {
        Some((code, at)) => {
            counters.outcome(code);
            if (200..300).contains(&code) {
                counters.invite_to_answer.record(at);
            }
        }
        // No final response inside the wait: a node that never answered.
        None => counters.outcome(0),
    }
    if !answered {
        return;
    }

    // Hold the call with a tone each way, then report what this leg
    // heard of the tone coming back.
    call.media().play_tone(opts.tone_hz, opts.call_secs * 1000);
    let held = tokio::time::sleep(Duration::from_secs(opts.call_secs));
    tokio::pin!(held);
    let ended_early = tokio::select! {
        _ = &mut held => false,
        _ = call.ended(Duration::from_secs(opts.call_secs + 5)) => true,
    };
    let stats = call.media().stream_stats();
    counters.media(LegMedia {
        received: stats.received,
        sent: call.media().packets_sent(),
        lost: stats.lost,
        out_of_order: stats.out_of_order,
        duplicates: stats.duplicates,
        jitter_ms: stats.jitter_ms,
    });
    if ended_early || call.phase() != CallPhase::Answered {
        counters.dropped();
        return;
    }
    if let Err(e) = call.hangup().await {
        debug!(error = %e, "BYE failed");
    }
}

/// Offer calls at `cps` until `stop` says otherwise or `for_` elapses.
/// Returns when every call it placed has finished.
async fn offer_calls(
    phones: &Phones,
    opts: &Arc<RunOptions>,
    counters: &Arc<Counters>,
    cps: f64,
    for_: Duration,
    stop: &Arc<AtomicBool>,
) {
    if phones.callers.is_empty() || cps <= 0.0 {
        return;
    }
    let gap = Duration::from_secs_f64(1.0 / cps);
    let in_flight = Arc::new(tokio::sync::Semaphore::new(opts.max_in_flight));
    let deadline = Instant::now() + for_;
    let mut ticker = tokio::time::interval(gap);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Burst);
    let mut next = 0u32;
    let mut tasks = Vec::new();
    while Instant::now() < deadline && !stop.load(Ordering::Relaxed) {
        ticker.tick().await;
        let i = next % opts.users.max(1);
        next = next.wrapping_add(1);
        let Ok(permit) = Arc::clone(&in_flight).acquire_owned().await else {
            break;
        };
        let phone = Arc::clone(&phones.callers[i as usize]);
        let destination = destination_for(opts, i);
        let opts = Arc::clone(opts);
        let counters = Arc::clone(counters);
        tasks.push(tokio::spawn(async move {
            one_call(phone, destination, opts, counters).await;
            drop(permit);
        }));
    }
    for task in tasks {
        let _ = task.await;
    }
}

/// Where caller `i` calls: the destination template with `{n}`, `{i}`
/// and `{callee}` filled in.
fn destination_for(opts: &RunOptions, i: u32) -> String {
    let mut destination = opts.target.destination_for(i);
    destination = destination.replace("{i}", &i.to_string());
    if let Some(callee) = Phones::callee_target(opts, i) {
        destination = destination.replace("{callee}", &callee);
    }
    destination
}

/// Run a profile and report what it measured.
pub async fn run(opts: RunOptions) -> Result<Report> {
    let opts = Arc::new(opts);
    let counters = Arc::new(Counters::default());
    let sampler = Sampler::start(
        opts.target.metrics_url.clone(),
        opts.target.pid,
        opts.sample_interval,
    );
    let started_at = chrono::Utc::now().to_rfc3339();
    let began = Instant::now();

    info!(
        profile = %opts.profile,
        users = opts.users,
        duration_secs = opts.duration.as_secs(),
        "starting phones"
    );
    let phones = Phones::start(&opts, &counters).await?;
    let stop = Arc::new(AtomicBool::new(false));
    let mut rungs = Vec::new();

    // The phones that answer, for as long as the run offers calls.
    let answering_stop = Arc::new(AtomicBool::new(false));
    let answering: Vec<_> = phones
        .callees
        .iter()
        .map(|phone| {
            tokio::spawn(answer_calls(
                Arc::clone(phone),
                Arc::clone(&opts),
                Arc::clone(&counters),
                // Past the run's own deadline, so the last call placed
                // is still answered.
                Instant::now() + opts.duration + Duration::from_secs(opts.call_secs + 5),
                Arc::clone(&answering_stop),
            ))
        })
        .collect();

    match opts.profile {
        Profile::RegisterStorm => register_storm(&phones, &opts, &counters).await,
        Profile::Soak => soak(&phones, &opts, &counters).await,
        Profile::CallRamp => rungs = call_ramp(&phones, &opts, &counters, &stop).await,
        Profile::Mixed => mixed(&phones, &opts, &counters, &stop).await,
    }

    // Stop answering, and give the calls already answered their moment
    // to end so their media is counted.
    answering_stop.store(true, Ordering::Relaxed);
    for task in answering {
        if tokio::time::timeout(Duration::from_secs(opts.call_secs + 10), task)
            .await
            .is_err()
        {
            warn!("a phone was still answering when the run ended");
        }
    }
    let elapsed = began.elapsed();
    let measurements = counters.summarise(elapsed);
    let budgets = opts.budget.judge(&measurements);
    let samples = sampler.finish().await;
    Ok(Report {
        version: env!("CARGO_PKG_VERSION").to_string(),
        profile: opts.profile.to_string(),
        options: serde_json::to_value(opts.as_ref()).unwrap_or(serde_json::Value::Null),
        started_at,
        duration_secs: elapsed.as_secs_f64(),
        measurements,
        samples,
        rungs,
        budgets,
    })
}

/// Re-register every phone on its expiry until the run is over: what a
/// registrar meets from a subnet that has just come back, and then keeps
/// meeting.
async fn register_storm(phones: &Phones, opts: &Arc<RunOptions>, counters: &Arc<Counters>) {
    let deadline = Instant::now() + opts.duration;
    // Devices re-register a little before their registration lapses, as
    // a real one does; a quarter early is the usual habit.
    let every = Duration::from_secs((opts.register_expiry as u64 * 3 / 4).max(1));
    while Instant::now() + every < deadline {
        tokio::time::sleep(every).await;
        phones.register_all(opts, counters).await;
    }
    let left = deadline.saturating_duration_since(Instant::now());
    if !left.is_zero() {
        tokio::time::sleep(left).await;
    }
}

/// Hold `users` calls up for the whole run: each caller places a call,
/// holds it with a tone for `call_secs`, hangs up and places another.
/// What the node is asked for here is not a rate but a population —
/// which is what memory per call, media quality over time and a session
/// timer's opinion are measured against.
async fn soak(phones: &Phones, opts: &Arc<RunOptions>, counters: &Arc<Counters>) {
    let deadline = Instant::now() + opts.duration;
    let mut tasks = Vec::new();
    for i in 0..opts.users {
        let phone = Arc::clone(&phones.callers[i as usize]);
        let destination = destination_for(opts, i);
        let opts = Arc::clone(opts);
        let counters = Arc::clone(counters);
        tasks.push(tokio::spawn(async move {
            while Instant::now() < deadline {
                one_call(
                    Arc::clone(&phone),
                    destination.clone(),
                    Arc::clone(&opts),
                    Arc::clone(&counters),
                )
                .await;
            }
        }));
    }
    for task in tasks {
        let _ = task.await;
    }
}

/// Climb the ladder: each rung offers more calls per second than the
/// last, and the run stops at the first rung that refuses too many or
/// answers too slowly. The report names every rung, so the last one that
/// held is the node's number.
async fn call_ramp(
    phones: &Phones,
    opts: &Arc<RunOptions>,
    counters: &Arc<Counters>,
    stop: &Arc<AtomicBool>,
) -> Vec<Rung> {
    let mut rungs = Vec::new();
    let deadline = Instant::now() + opts.duration;
    let mut cps = opts.cps;
    while cps <= opts.cps_max && Instant::now() < deadline {
        let rung_counters = Arc::new(Counters::default());
        let began = Instant::now();
        let left = deadline.saturating_duration_since(began);
        let for_ = Duration::from_secs(opts.rung_secs).min(left);
        info!(cps, secs = for_.as_secs(), "rung");
        offer_calls(phones, opts, &rung_counters, cps, for_, stop).await;
        let measurements = rung_counters.summarise(began.elapsed());
        // The run's totals are every rung together.
        counters.absorb(&rung_counters);
        let why = rung_verdict(opts, &measurements);
        rungs.push(Rung {
            offered_cps: cps,
            measurements,
            stopped_because: why.clone(),
        });
        if why.is_some() {
            break;
        }
        cps += opts.cps_step;
    }
    rungs
}

/// Why a rung failed, if it did.
fn rung_verdict(opts: &RunOptions, m: &Measurements) -> Option<String> {
    if m.offered == 0 {
        return Some("no calls were offered".to_string());
    }
    if m.refused_fraction > opts.stop_refused_fraction {
        return Some(format!(
            "{:.1}% refused, past {:.1}%",
            m.refused_fraction * 100.0,
            opts.stop_refused_fraction * 100.0
        ));
    }
    if m.invite_to_answer.count > 0 && m.invite_to_answer.p99_ms > opts.stop_p99_ms {
        return Some(format!(
            "p99 INVITE→200 {:.0}ms, past {:.0}ms",
            m.invite_to_answer.p99_ms, opts.stop_p99_ms
        ));
    }
    if m.dropped > 0 {
        return Some(format!("{} calls ended by themselves", m.dropped));
    }
    None
}

/// Calls and registrations at once, which is what a PBX actually sees: a
/// steady offered rate with the devices re-registering underneath it.
async fn mixed(
    phones: &Phones,
    opts: &Arc<RunOptions>,
    counters: &Arc<Counters>,
    stop: &Arc<AtomicBool>,
) {
    let calls = offer_calls(phones, opts, counters, opts.cps, opts.duration, stop);
    let registrations = register_storm(phones, opts, counters);
    tokio::join!(calls, registrations);
}
