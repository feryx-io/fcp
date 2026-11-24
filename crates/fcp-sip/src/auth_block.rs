//! Blocking a source that keeps failing digest authentication
//! (`docs/EDGE_SECURITY.md` §2.8).
//!
//! The per-source rate bounds how fast one address may talk, not what it
//! says. On D8 a credential scanner sent one node 35 million REGISTERs in
//! 17 days, about 24 a second and so under the rate, guessing passwords
//! for names it made up. Every one was parsed, looked up and answered.
//!
//! A source that fails authentication `max_failures` times within a minute
//! is blocked for `block_secs`: every message from it is dropped at the
//! node's inbound handler, before the parser ([`admit`]), and it is told
//! nothing.
//!
//! What counts as a failure is narrow, because a block refuses everything
//! behind an address — a whole office behind one NAT:
//!
//! - Only credentials that answer a challenge **this node issued** and that
//!   is still live. A request with no credentials is the first half of every
//!   ordinary sign-in, and a nonce from elsewhere could come from anywhere.
//!   Holding a nonce of ours means the source received our 401, so a forged
//!   UDP source address cannot get a stranger's phone blocked.
//! - Wrong credentials: a digest that does not match, a user who does not
//!   exist, another realm. Right credentials on a stale nonce are not a
//!   failure.
//! - Never a source exempt from the per-source rate (`crate::ingress`): an
//!   SBC, a trunk, a trusted edge or another node carries other people's
//!   traffic, and one of them mistyping a password must not cut off the
//!   rest.
//!
//! A phone with a wrong password retries every few minutes at most, which is
//! far below the default of 20 failures a minute.

use dashmap::DashMap;
use std::net::IpAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use tracing::{info, warn};

/// Failed authentications in a minute from one source before it is
/// blocked, when the configuration says nothing.
pub const DEFAULT_MAX_AUTH_FAILURES: u32 = 20;

/// How long a source stays blocked, when the configuration says nothing.
pub const DEFAULT_AUTH_BLOCK_SECS: u64 = 900;

/// For serde: [`DEFAULT_MAX_AUTH_FAILURES`].
pub fn default_max_auth_failures() -> u32 {
    DEFAULT_MAX_AUTH_FAILURES
}

/// For serde: [`DEFAULT_AUTH_BLOCK_SECS`].
pub fn default_auth_block_secs() -> u64 {
    DEFAULT_AUTH_BLOCK_SECS
}

/// The window failures are counted over.
const WINDOW: Duration = Duration::from_secs(60);

/// Sources counted at once. Reaching it drops the counts that have aged
/// out; a flood of distinct sources each failing once costs no more.
const MAX_TRACKED: usize = 100_000;

/// The longest block the configuration may ask for: a day.
pub const MAX_AUTH_BLOCK_SECS: u64 = 86_400;

/// Check the two settings, so a typo is a startup error.
pub fn validate(max_failures: u32, block_secs: u64) -> Result<(), String> {
    if max_failures != 0 && !(1..=MAX_AUTH_BLOCK_SECS).contains(&block_secs) {
        return Err(format!(
            "auth_block_secs must be between 1 and {MAX_AUTH_BLOCK_SECS}"
        ));
    }
    Ok(())
}

/// One process's counts and blocks.
pub struct AuthBlock {
    max_failures: u32,
    block: Duration,
    /// Failures in the current window, per source: when it began, how many.
    failures: DashMap<IpAddr, (Instant, u32)>,
    /// Blocked sources and when each block ends.
    blocked: DashMap<IpAddr, Instant>,
    blocks_total: AtomicU64,
    dropped_total: AtomicU64,
}

impl AuthBlock {
    fn new(max_failures: u32, block: Duration) -> Self {
        Self {
            max_failures,
            block,
            failures: DashMap::new(),
            blocked: DashMap::new(),
            blocks_total: AtomicU64::new(0),
            dropped_total: AtomicU64::new(0),
        }
    }

    /// Record a failed authentication from `ip`; `true` when this one
    /// blocked it.
    fn note_failure(&self, ip: IpAddr, now: Instant) -> bool {
        let ip = unmap(ip);
        if self.blocked.contains_key(&ip) || crate::ingress::is_exempt(ip) {
            return false;
        }
        if self.failures.len() >= MAX_TRACKED {
            self.failures
                .retain(|_, (since, _)| now.duration_since(*since) < WINDOW);
            if self.failures.len() >= MAX_TRACKED && !self.failures.contains_key(&ip) {
                return false;
            }
        }
        let count = {
            let mut entry = self.failures.entry(ip).or_insert((now, 0));
            if now.duration_since(entry.0) >= WINDOW {
                *entry = (now, 0);
            }
            entry.1 += 1;
            entry.1
        };
        if count < self.max_failures {
            return false;
        }
        self.failures.remove(&ip);
        self.blocked.insert(ip, now + self.block);
        self.blocks_total.fetch_add(1, Ordering::Relaxed);
        warn!(
            source = %ip,
            failures = count,
            block_secs = self.block.as_secs(),
            "Blocking a source: it failed SIP authentication too often; \
             nothing from it is read until the block ends"
        );
        true
    }

    /// Whether `ip` is blocked now. A block that has ended is lifted here.
    fn is_blocked(&self, ip: IpAddr, now: Instant) -> bool {
        let ip = unmap(ip);
        let Some(until) = self.blocked.get(&ip).map(|until| *until) else {
            return false;
        };
        if now < until {
            return true;
        }
        if self
            .blocked
            .remove_if(&ip, |_, until| now >= *until)
            .is_some()
        {
            info!(source = %ip, "A source's authentication block has ended");
        }
        false
    }

    /// The sources blocked now, with the seconds left on each.
    fn blocked_now(&self, now: Instant) -> Vec<(IpAddr, u64)> {
        let mut list: Vec<_> = self
            .blocked
            .iter()
            .filter(|entry| *entry.value() > now)
            .map(|entry| (*entry.key(), entry.value().duration_since(now).as_secs()))
            .collect();
        list.sort();
        list
    }
}

static INSTALLED: RwLock<Option<Arc<AuthBlock>>> = RwLock::new(None);

fn installed() -> Option<Arc<AuthBlock>> {
    INSTALLED.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Turn blocking on for this process: `max_failures` failures from one
/// source within a minute block it for `block_secs`. `0` failures is off.
/// Replaces any earlier setting, and its blocks.
pub fn install(max_failures: u32, block_secs: u64) {
    if max_failures == 0 {
        *INSTALLED.write().unwrap_or_else(|e| e.into_inner()) = None;
        info!(
            "Sources failing SIP authentication are not blocked (max_auth_failures_per_source = 0)"
        );
        return;
    }
    let guard = Arc::new(AuthBlock::new(
        max_failures,
        Duration::from_secs(block_secs.clamp(1, MAX_AUTH_BLOCK_SECS)),
    ));
    *INSTALLED.write().unwrap_or_else(|e| e.into_inner()) = Some(guard);
    info!(
        max_failures,
        block_secs, "Sources failing SIP authentication this often in a minute are blocked"
    );
}

/// Record a failed authentication from `source` (see the module's notes on
/// what counts).
pub fn note_failure(source: IpAddr) {
    if let Some(guard) = installed() {
        guard.note_failure(source, Instant::now());
    }
}

/// Whether `source` is blocked now.
pub fn is_blocked(source: IpAddr) -> bool {
    installed().is_some_and(|guard| guard.is_blocked(source, Instant::now()))
}

/// Whether a message from `source` is read at all: `false` for a blocked
/// source, whose message is counted as dropped. Asked by every inbound
/// handler before it parses anything.
pub fn admit(source: IpAddr) -> bool {
    let Some(guard) = installed() else {
        return true;
    };
    if guard.is_blocked(source, Instant::now()) {
        guard.dropped_total.fetch_add(1, Ordering::Relaxed);
        return false;
    }
    true
}

/// What a node reports about its blocks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// The sources blocked now, with the seconds left on each.
    pub blocked: Vec<(IpAddr, u64)>,
    /// Blocks made since the process started.
    pub blocks_total: u64,
    /// Messages dropped because their source was blocked.
    pub dropped_total: u64,
}

/// This process's blocks; `None` when blocking is off.
pub fn snapshot() -> Option<Snapshot> {
    installed().map(|guard| Snapshot {
        blocked: guard.blocked_now(Instant::now()),
        blocks_total: guard.blocks_total.load(Ordering::Relaxed),
        dropped_total: guard.dropped_total.load(Ordering::Relaxed),
    })
}

/// The same as JSON, for a node's `/status`; `null` when blocking is off.
pub fn json() -> serde_json::Value {
    match snapshot() {
        None => serde_json::Value::Null,
        Some(Snapshot {
            blocked,
            blocks_total,
            dropped_total,
        }) => serde_json::json!({
            "blocked": blocked
                .into_iter()
                .map(|(source, secs_left)| serde_json::json!({
                    "source": source.to_string(),
                    "secs_left": secs_left,
                }))
                .collect::<Vec<_>>(),
            "blocks_total": blocks_total,
            "dropped_total": dropped_total,
        }),
    }
}

/// The same in Prometheus text, with `labels` on every sample; empty when
/// blocking is off.
pub fn prometheus(labels: &str) -> String {
    let Some(Snapshot {
        blocked,
        blocks_total: blocks,
        dropped_total: dropped,
    }) = snapshot()
    else {
        return String::new();
    };
    format!(
        "# HELP fcp_sip_auth_blocked_sources Sources blocked now for failing SIP authentication.\n\
         # TYPE fcp_sip_auth_blocked_sources gauge\n\
         fcp_sip_auth_blocked_sources{{{labels}}} {}\n\
         # HELP fcp_sip_auth_blocks_total Sources blocked for failing SIP authentication.\n\
         # TYPE fcp_sip_auth_blocks_total counter\n\
         fcp_sip_auth_blocks_total{{{labels}}} {blocks}\n\
         # HELP fcp_sip_auth_blocked_dropped_total Messages dropped unread from blocked sources.\n\
         # TYPE fcp_sip_auth_blocked_dropped_total counter\n\
         fcp_sip_auth_blocked_dropped_total{{{labels}}} {dropped}\n",
        blocked.len()
    )
}

/// An IPv4 peer on a dual-stack socket arrives mapped into IPv6.
fn unmap(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(ip),
        v4 => v4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn a_source_is_blocked_at_the_limit_and_freed_after_the_block() {
        let guard = AuthBlock::new(3, Duration::from_secs(10));
        let t0 = Instant::now();
        let scanner = ip("198.51.100.7");
        assert!(!guard.note_failure(scanner, t0));
        assert!(!guard.note_failure(scanner, t0));
        assert!(!guard.is_blocked(scanner, t0));
        assert!(guard.note_failure(scanner, t0), "the third failure blocks");
        assert!(guard.is_blocked(scanner, t0 + Duration::from_secs(9)));
        assert!(
            !guard.is_blocked(ip("198.51.100.8"), t0),
            "only that source"
        );
        assert_eq!(guard.blocked_now(t0), vec![(scanner, 10)]);
        assert!(!guard.is_blocked(scanner, t0 + Duration::from_secs(10)));
        assert!(guard.blocked_now(t0 + Duration::from_secs(10)).is_empty());
        // Freed with a clean count.
        assert!(!guard.note_failure(scanner, t0 + Duration::from_secs(11)));
    }

    #[test]
    fn failures_spread_wider_than_a_minute_never_block() {
        let guard = AuthBlock::new(3, Duration::from_secs(10));
        let t0 = Instant::now();
        let phone = ip("203.0.113.5");
        // A phone with a wrong password, retrying every 30 seconds.
        for n in 0..20u64 {
            assert!(!guard.note_failure(phone, t0 + Duration::from_secs(30 * n)));
        }
        assert!(!guard.is_blocked(phone, t0 + Duration::from_secs(600)));
    }

    #[test]
    fn an_ipv4_source_mapped_into_ipv6_is_the_same_source() {
        let guard = AuthBlock::new(2, Duration::from_secs(10));
        let t0 = Instant::now();
        guard.note_failure(ip("::ffff:198.51.100.9"), t0);
        guard.note_failure(ip("198.51.100.9"), t0);
        assert!(guard.is_blocked(ip("::ffff:198.51.100.9"), t0));
        assert!(guard.is_blocked(ip("198.51.100.9"), t0));
    }

    #[test]
    fn a_block_longer_than_a_day_is_refused() {
        assert!(validate(20, 900).is_ok());
        assert!(validate(20, 0).is_err());
        assert!(validate(20, MAX_AUTH_BLOCK_SECS + 1).is_err());
        assert!(validate(0, 0).is_ok(), "off needs no block length");
    }
}
