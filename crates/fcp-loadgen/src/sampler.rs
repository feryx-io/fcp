//! What the node says about itself while the run goes on: its `/metrics`,
//! and — when it is a process on this machine — its resident memory and
//! CPU (`docs/CAPACITY_AND_SURVIVABILITY.md` §8).
//!
//! Sampling is best-effort by design: a node that is too busy to answer
//! `/metrics` for one tick is exactly the node a load run is trying to
//! describe, so a failed scrape leaves a gap rather than ending the run.

use fcp_testkit::smoke::http_get;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

/// One moment of the node's own view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sample {
    /// Seconds since the run began.
    pub at_secs: f64,
    /// Resident set size, when the node's process is known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rss_bytes: Option<u64>,
    /// CPU used since the previous sample, as a percentage of one core.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_percent: Option<f64>,
    /// The unlabelled `fcp_*` and `sip_*` series from `/metrics`. A
    /// labelled series is summed across its labels, which is what a
    /// single-node run wants from `…_total{outcome="…"}`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metrics: BTreeMap<String, f64>,
}

/// Samples the node every `interval` until it is stopped.
pub struct Sampler {
    samples: Arc<Mutex<Vec<Sample>>>,
    stop: tokio::sync::watch::Sender<bool>,
}

impl Sampler {
    /// Start sampling. With neither a metrics URL nor a pid there is
    /// nothing to sample, and the sampler holds no task.
    pub fn start(metrics_url: Option<String>, pid: Option<u32>, interval: Duration) -> Self {
        let samples = Arc::new(Mutex::new(Vec::new()));
        let (stop, mut stopped) = tokio::sync::watch::channel(false);
        if metrics_url.is_none() && pid.is_none() {
            return Self { samples, stop };
        }
        let out = Arc::clone(&samples);
        tokio::spawn(async move {
            let began = Instant::now();
            let mut cpu = CpuUsage::default();
            let mut ticker = tokio::time::interval(interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    _ = stopped.changed() => return,
                    _ = ticker.tick() => {}
                }
                let metrics = match &metrics_url {
                    Some(url) => scrape(url).await,
                    None => BTreeMap::new(),
                };
                let (rss_bytes, cpu_percent) = match pid {
                    Some(pid) => (rss_bytes(pid), cpu.since(pid)),
                    None => (None, None),
                };
                out.lock().await.push(Sample {
                    at_secs: began.elapsed().as_secs_f64(),
                    rss_bytes,
                    cpu_percent,
                    metrics,
                });
            }
        });
        Self { samples, stop }
    }

    /// Stop sampling and take what was collected.
    pub async fn finish(self) -> Vec<Sample> {
        let _ = self.stop.send(true);
        // Let the task see the stop before the samples are taken.
        tokio::task::yield_now().await;
        self.samples.lock().await.clone()
    }
}

/// Scrape a Prometheus text endpoint into name → value, summing a series
/// that carries labels. A scrape that fails is an empty map: the run goes
/// on without that tick.
async fn scrape(url: &str) -> BTreeMap<String, f64> {
    let Ok((200, body)) = http_get(url).await else {
        return BTreeMap::new();
    };
    parse_prometheus(&body)
}

/// The value lines of a Prometheus exposition, summed per metric name.
pub fn parse_prometheus(body: &str) -> BTreeMap<String, f64> {
    let mut out: BTreeMap<String, f64> = BTreeMap::new();
    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // `name{label="v",…} 1.5` or `name 1.5`; a trailing timestamp is
        // ignored, as the sample's own time is what a run cares about.
        let (name_part, rest) = match line.split_once(|c: char| c.is_whitespace()) {
            Some(x) => x,
            None => continue,
        };
        let name = name_part
            .split_once('{')
            .map(|(n, _)| n)
            .unwrap_or(name_part);
        let Ok(value) = rest.split_whitespace().next().unwrap_or("").parse::<f64>() else {
            continue;
        };
        *out.entry(name.to_string()).or_default() += value;
    }
    out
}

/// A process's resident set size, from `/proc/<pid>/statm` (pages).
fn rss_bytes(pid: u32) -> Option<u64> {
    let statm = std::fs::read_to_string(format!("/proc/{pid}/statm")).ok()?;
    let pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
    Some(pages * page_size())
}

fn page_size() -> u64 {
    // 4 KiB everywhere this runs; reading it is not worth a libc call.
    4096
}

/// CPU used between samples, as a percentage of one core.
#[derive(Default)]
struct CpuUsage {
    last: Option<(f64, Instant)>,
}

impl CpuUsage {
    fn since(&mut self, pid: u32) -> Option<f64> {
        let ticks = process_ticks(pid)?;
        let now = Instant::now();
        let seconds = ticks / clock_ticks_per_sec();
        let percent = match self.last {
            Some((last_seconds, at)) => {
                let wall = now.duration_since(at).as_secs_f64().max(0.001);
                Some((seconds - last_seconds) / wall * 100.0)
            }
            None => None,
        };
        self.last = Some((seconds, now));
        percent
    }
}

/// User + system time of a process, in clock ticks, from `/proc/<pid>/stat`.
fn process_ticks(pid: u32) -> Option<f64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // Fields after the command, which may itself contain spaces inside
    // parentheses: everything is counted from the closing one.
    let after = stat.rsplit_once(')')?.1;
    let fields: Vec<&str> = after.split_whitespace().collect();
    // utime and stime are fields 14 and 15 of the whole line; the first
    // field after ')' is state, which is field 3.
    let utime: f64 = fields.get(11)?.parse().ok()?;
    let stime: f64 = fields.get(12)?.parse().ok()?;
    Some(utime + stime)
}

fn clock_ticks_per_sec() -> f64 {
    100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_labelled_series_is_summed_across_its_labels() {
        let body = r#"
# HELP fcp_calls_total Calls
# TYPE fcp_calls_total counter
fcp_calls_total{outcome="answered",node="a"} 12
fcp_calls_total{outcome="refused",node="a"} 3
fcp_active_calls 7
sip_transactions_total 1.5e2
"#;
        let m = parse_prometheus(body);
        assert_eq!(m["fcp_calls_total"], 15.0, "both label sets, one number");
        assert_eq!(m["fcp_active_calls"], 7.0);
        assert_eq!(m["sip_transactions_total"], 150.0);
        assert!(!m.contains_key("# HELP"), "comments are not series");
    }

    #[test]
    fn a_line_without_a_number_is_skipped_rather_than_failing_the_scrape() {
        let m = parse_prometheus("fcp_broken not_a_number\nfcp_ok 1\nnothing\n");
        assert_eq!(m.len(), 1);
        assert_eq!(m["fcp_ok"], 1.0);
    }

    /// This process is one the sampler can read, which is enough to know
    /// the /proc paths are right.
    #[test]
    fn a_running_process_has_a_resident_size_and_cpu_time() {
        let me = std::process::id();
        assert!(rss_bytes(me).unwrap_or(0) > 0);
        assert!(process_ticks(me).is_some());
        let mut cpu = CpuUsage::default();
        assert_eq!(cpu.since(me), None, "the first sample has nothing to span");
        assert!(cpu.since(me).is_some());
    }

    #[tokio::test]
    async fn a_sampler_with_nothing_to_watch_collects_nothing() {
        let sampler = Sampler::start(None, None, Duration::from_millis(10));
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(sampler.finish().await.is_empty());
    }

    #[tokio::test]
    async fn a_sampler_watching_this_process_collects_samples() {
        let sampler = Sampler::start(None, Some(std::process::id()), Duration::from_millis(10));
        tokio::time::sleep(Duration::from_millis(50)).await;
        let samples = sampler.finish().await;
        assert!(!samples.is_empty(), "something was sampled");
        assert!(samples[0].rss_bytes.unwrap_or(0) > 0);
    }
}
