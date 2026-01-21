//! DTMF digit collector for Gather nodes.
//!
//! Accumulates DTMF digits with timeout and finish-on-key logic.

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, RwLock};
use tokio::time::{timeout, Instant};
use tracing::{debug, info, warn};

/// Result of DTMF collection
#[derive(Debug, Clone)]
pub enum CollectionResult {
    /// Digits collected successfully
    Complete(String),

    /// Collection timed out
    Timeout(String),

    /// Collection cancelled
    Cancelled,
}

/// DTMF collector state
#[derive(Debug)]
struct CollectorState {
    /// Accumulated digits
    digits: String,

    /// Maximum number of digits to collect
    max_digits: usize,

    /// Finish-on-key character (e.g., '#')
    finish_on_key: Option<char>,

    /// Whether collection is active
    active: bool,

    /// Start time of collection
    start_time: Instant,
}

/// DTMF digit collector
pub struct DtmfCollector {
    /// Internal state
    state: Arc<RwLock<CollectorState>>,

    /// Channel for receiving DTMF digits
    digit_rx: Arc<RwLock<Option<mpsc::UnboundedReceiver<char>>>>,

    /// Channel for sending DTMF digits (to be cloned and used by media backend)
    digit_tx: mpsc::UnboundedSender<char>,
}

impl DtmfCollector {
    /// Create a new DTMF collector
    pub fn new(max_digits: usize, finish_on_key: Option<char>) -> Self {
        let (digit_tx, digit_rx) = mpsc::unbounded_channel();

        let state = CollectorState {
            digits: String::new(),
            max_digits,
            finish_on_key,
            active: true,
            start_time: Instant::now(),
        };

        Self {
            state: Arc::new(RwLock::new(state)),
            digit_rx: Arc::new(RwLock::new(Some(digit_rx))),
            digit_tx,
        }
    }

    /// Get a sender for pushing DTMF digits into this collector
    pub fn get_sender(&self) -> mpsc::UnboundedSender<char> {
        self.digit_tx.clone()
    }

    /// Start collecting digits with a timeout
    ///
    /// Returns when:
    /// - max_digits collected
    /// - finish_on_key pressed
    /// - timeout expires
    /// - collection cancelled
    pub async fn collect(&self, timeout_duration: Duration) -> CollectionResult {
        let mut digit_rx_guard = self.digit_rx.write().await;
        let mut digit_rx = match digit_rx_guard.take() {
            Some(rx) => rx,
            None => {
                warn!("DTMF collector already collecting");
                return CollectionResult::Cancelled;
            }
        };
        drop(digit_rx_guard);

        // Extract values before tracing to avoid Send issues
        let state_guard = self.state.read().await;
        let max_digits = state_guard.max_digits;
        let finish_on_key = state_guard.finish_on_key;
        drop(state_guard);

        info!(
            max_digits = max_digits,
            finish_on_key = ?finish_on_key,
            timeout_ms = timeout_duration.as_millis(),
            "Starting DTMF collection"
        );

        // Collect digits until timeout or completion
        let result = timeout(timeout_duration, async {
            loop {
                match digit_rx.recv().await {
                    Some(digit) => {
                        let mut state = self.state.write().await;

                        if !state.active {
                            debug!("DTMF collection cancelled");
                            return CollectionResult::Cancelled;
                        }

                        // Validate digit
                        if !is_valid_dtmf_digit(digit) {
                            warn!(digit = %digit, "Invalid DTMF digit, ignoring");
                            continue;
                        }

                        // Add digit to accumulated string
                        state.digits.push(digit);

                        debug!(
                            digit = %digit,
                            collected = %state.digits,
                            count = state.digits.len(),
                            "DTMF digit collected"
                        );

                        // Check if we should finish
                        if state.finish_on_key == Some(digit) {
                            let digits = state.digits.clone();
                            state.active = false;
                            info!(
                                digits = %digits,
                                elapsed_ms = state.start_time.elapsed().as_millis(),
                                "DTMF collection complete (finish-on-key)"
                            );
                            return CollectionResult::Complete(digits);
                        }

                        if state.digits.len() >= state.max_digits {
                            let digits = state.digits.clone();
                            state.active = false;
                            info!(
                                digits = %digits,
                                elapsed_ms = state.start_time.elapsed().as_millis(),
                                "DTMF collection complete (max digits)"
                            );
                            return CollectionResult::Complete(digits);
                        }
                    }
                    None => {
                        // Channel closed
                        let state = self.state.read().await;
                        let digits = state.digits.clone();
                        info!(
                            digits = %digits,
                            "DTMF collection channel closed"
                        );
                        return CollectionResult::Complete(digits);
                    }
                }
            }
        })
        .await;

        match result {
            Ok(collection_result) => collection_result,
            Err(_) => {
                // Timeout
                let mut state = self.state.write().await;
                let digits = state.digits.clone();
                state.active = false;
                info!(
                    digits = %digits,
                    elapsed_ms = state.start_time.elapsed().as_millis(),
                    "DTMF collection timeout"
                );
                CollectionResult::Timeout(digits)
            }
        }
    }

    /// Cancel the collection
    pub async fn cancel(&self) {
        let mut state = self.state.write().await;
        state.active = false;
        debug!("DTMF collection cancelled");
    }

    /// Get currently collected digits
    pub async fn get_digits(&self) -> String {
        self.state.read().await.digits.clone()
    }
}

/// Check if a character is a valid DTMF digit
fn is_valid_dtmf_digit(c: char) -> bool {
    matches!(c, '0'..='9' | '*' | '#' | 'A'..='D')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_dtmf_digits() {
        assert!(is_valid_dtmf_digit('0'));
        assert!(is_valid_dtmf_digit('9'));
        assert!(is_valid_dtmf_digit('*'));
        assert!(is_valid_dtmf_digit('#'));
        assert!(is_valid_dtmf_digit('A'));
        assert!(is_valid_dtmf_digit('D'));
        assert!(!is_valid_dtmf_digit('E'));
        assert!(!is_valid_dtmf_digit('x'));
    }

    #[tokio::test]
    async fn test_collect_max_digits() {
        let collector = DtmfCollector::new(3, None);
        let sender = collector.get_sender();

        // Send digits in background
        tokio::spawn(async move {
            sender.send('1').unwrap();
            sender.send('2').unwrap();
            sender.send('3').unwrap();
        });

        let result = collector.collect(Duration::from_secs(5)).await;
        match result {
            CollectionResult::Complete(digits) => {
                assert_eq!(digits, "123");
            }
            _ => panic!("Expected Complete result"),
        }
    }

    #[tokio::test]
    async fn test_collect_finish_on_key() {
        let collector = DtmfCollector::new(10, Some('#'));
        let sender = collector.get_sender();

        // Send digits in background
        tokio::spawn(async move {
            sender.send('1').unwrap();
            sender.send('2').unwrap();
            sender.send('#').unwrap();
        });

        let result = collector.collect(Duration::from_secs(5)).await;
        match result {
            CollectionResult::Complete(digits) => {
                assert_eq!(digits, "12#");
            }
            _ => panic!("Expected Complete result"),
        }
    }

    #[tokio::test]
    async fn test_collect_timeout() {
        let collector = DtmfCollector::new(10, None);
        let sender = collector.get_sender();

        // Send only one digit
        tokio::spawn(async move {
            sender.send('1').unwrap();
        });

        let result = collector.collect(Duration::from_millis(100)).await;
        match result {
            CollectionResult::Timeout(digits) => {
                assert_eq!(digits, "1");
            }
            _ => panic!("Expected Timeout result"),
        }
    }
}
