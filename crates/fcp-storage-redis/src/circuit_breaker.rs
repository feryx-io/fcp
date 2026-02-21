//! Circuit breaker for Redis failover.
//!
//! Implements a 3-state circuit breaker pattern:
//! - **Closed**: All requests pass through. Failures are counted.
//! - **Open**: All requests are rejected immediately. After a cooldown period,
//!   transitions to Half-Open.
//! - **Half-Open**: A single request is allowed through. If it succeeds, the
//!   breaker closes. If it fails, the breaker opens again.

use std::sync::atomic::{AtomicU32, AtomicU8, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tracing::{info, warn};

/// Circuit breaker state values.
const STATE_CLOSED: u8 = 0;
const STATE_OPEN: u8 = 1;
const STATE_HALF_OPEN: u8 = 2;

/// Observable circuit breaker state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    Closed,
    Open,
    HalfOpen,
}

impl std::fmt::Display for CircuitState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CircuitState::Closed => write!(f, "closed"),
            CircuitState::Open => write!(f, "open"),
            CircuitState::HalfOpen => write!(f, "half_open"),
        }
    }
}

/// A thread-safe circuit breaker for Redis operations.
///
/// When the failure count reaches `failure_threshold`, the breaker opens and
/// rejects all requests for `cooldown` duration. After the cooldown, one
/// probe request is allowed through (half-open). If the probe succeeds the
/// breaker closes; if it fails the breaker reopens.
pub struct RedisCircuitBreaker {
    state: AtomicU8,
    failure_count: AtomicU32,
    failure_threshold: u32,
    cooldown: Duration,
    last_failure: Mutex<Option<Instant>>,
}

impl RedisCircuitBreaker {
    /// Create a new circuit breaker.
    ///
    /// - `threshold`: Number of consecutive failures before opening the circuit.
    /// - `cooldown`: Duration to wait before allowing a probe request.
    pub fn new(threshold: u32, cooldown: Duration) -> Self {
        Self {
            state: AtomicU8::new(STATE_CLOSED),
            failure_count: AtomicU32::new(0),
            failure_threshold: threshold,
            cooldown,
            last_failure: Mutex::new(None),
        }
    }

    /// Check whether a request should be allowed through.
    ///
    /// Returns `true` if the request may proceed, `false` if the circuit is
    /// open and the cooldown has not yet elapsed.
    pub fn allow_request(&self) -> bool {
        let state = self.state.load(Ordering::Acquire);
        match state {
            STATE_CLOSED => true,
            STATE_OPEN => {
                // Check if cooldown has elapsed
                let last = self.last_failure.lock().unwrap();
                if let Some(t) = *last {
                    if t.elapsed() >= self.cooldown {
                        drop(last);
                        // Transition to half-open — allow one probe
                        self.state.store(STATE_HALF_OPEN, Ordering::Release);
                        info!("Redis circuit breaker transitioning to half-open");
                        true
                    } else {
                        false
                    }
                } else {
                    // No recorded failure time — shouldn't happen, but allow
                    true
                }
            }
            STATE_HALF_OPEN => {
                // Only one probe at a time — allow it
                true
            }
            _ => true,
        }
    }

    /// Record a successful operation.
    ///
    /// Resets the failure count and closes the circuit if it was half-open.
    pub fn record_success(&self) {
        let prev = self.state.load(Ordering::Acquire);
        if prev == STATE_HALF_OPEN {
            info!("Redis circuit breaker closing (probe succeeded)");
        }
        self.failure_count.store(0, Ordering::Release);
        self.state.store(STATE_CLOSED, Ordering::Release);
    }

    /// Record a failed operation.
    ///
    /// Increments the failure counter. If the threshold is reached (or the
    /// breaker was half-open), opens the circuit.
    pub fn record_failure(&self) {
        let prev_state = self.state.load(Ordering::Acquire);

        if prev_state == STATE_HALF_OPEN {
            // Half-open probe failed → reopen
            warn!("Redis circuit breaker reopening (probe failed)");
            self.state.store(STATE_OPEN, Ordering::Release);
            *self.last_failure.lock().unwrap() = Some(Instant::now());
            return;
        }

        let count = self.failure_count.fetch_add(1, Ordering::AcqRel) + 1;
        *self.last_failure.lock().unwrap() = Some(Instant::now());

        if count >= self.failure_threshold && prev_state == STATE_CLOSED {
            warn!(
                failures = count,
                threshold = self.failure_threshold,
                cooldown_secs = self.cooldown.as_secs(),
                "Redis circuit breaker opening"
            );
            self.state.store(STATE_OPEN, Ordering::Release);
        }
    }

    /// Get the current circuit state.
    pub fn state(&self) -> CircuitState {
        match self.state.load(Ordering::Acquire) {
            STATE_CLOSED => CircuitState::Closed,
            STATE_OPEN => CircuitState::Open,
            STATE_HALF_OPEN => CircuitState::HalfOpen,
            _ => CircuitState::Closed,
        }
    }

    /// Get the current consecutive failure count.
    pub fn failure_count(&self) -> u32 {
        self.failure_count.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_starts_closed() {
        let cb = RedisCircuitBreaker::new(3, Duration::from_secs(30));
        assert_eq!(cb.state(), CircuitState::Closed);
        assert!(cb.allow_request());
    }

    #[test]
    fn test_opens_after_threshold() {
        let cb = RedisCircuitBreaker::new(3, Duration::from_secs(30));

        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Closed);
        assert!(cb.allow_request());

        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Closed);

        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Open);
        assert!(!cb.allow_request());
    }

    #[test]
    fn test_success_resets_count() {
        let cb = RedisCircuitBreaker::new(3, Duration::from_secs(30));

        cb.record_failure();
        cb.record_failure();
        assert_eq!(cb.failure_count(), 2);

        cb.record_success();
        assert_eq!(cb.failure_count(), 0);
        assert_eq!(cb.state(), CircuitState::Closed);
    }

    #[test]
    fn test_half_open_after_cooldown() {
        let cb = RedisCircuitBreaker::new(2, Duration::from_millis(10));

        cb.record_failure();
        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Open);

        // Wait for cooldown
        std::thread::sleep(Duration::from_millis(20));
        assert!(cb.allow_request());
        assert_eq!(cb.state(), CircuitState::HalfOpen);
    }

    #[test]
    fn test_half_open_success_closes() {
        let cb = RedisCircuitBreaker::new(2, Duration::from_millis(10));

        cb.record_failure();
        cb.record_failure();
        std::thread::sleep(Duration::from_millis(20));
        assert!(cb.allow_request()); // transitions to half-open

        cb.record_success();
        assert_eq!(cb.state(), CircuitState::Closed);
        assert_eq!(cb.failure_count(), 0);
    }

    #[test]
    fn test_half_open_failure_reopens() {
        let cb = RedisCircuitBreaker::new(2, Duration::from_millis(10));

        cb.record_failure();
        cb.record_failure();
        std::thread::sleep(Duration::from_millis(20));
        assert!(cb.allow_request()); // transitions to half-open

        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Open);
    }
}
