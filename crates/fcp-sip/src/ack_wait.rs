//! The caller's ACK for a 2xx the node sent itself. A BYE on that dialog
//! waits for it: a UAS must not send BYE before the ACK for its 2xx has
//! arrived, or its server transaction has timed out (RFC 3261 §15). A flow
//! that answered and hung up at once sent its BYE ahead of the ACK, and a
//! phone that took the BYE first never saw the call answered
//! (`docs/DEEP_TESTING.md` D8).

use std::time::Duration;

use dashmap::DashMap;
use fcp_b2bua_core::CallId;
use tokio::sync::watch;

/// How long a BYE waits for the ACK: 64·T1, an INVITE server
/// transaction's life (RFC 3261 §17.2.1).
pub const ACK_WAIT: Duration = Duration::from_secs(32);

/// 2xx responses the node sent that have not been ACKed, by call.
#[derive(Default)]
pub struct AckWaits {
    waits: DashMap<CallId, watch::Sender<bool>>,
}

impl AckWaits {
    pub fn new() -> Self {
        Self::default()
    }

    /// A 2xx for `call_id` is about to go out.
    pub fn expect(&self, call_id: CallId) {
        self.waits.insert(call_id, watch::channel(false).0);
    }

    /// The caller's ACK for `call_id` arrived.
    pub fn acked(&self, call_id: &CallId) {
        if let Some((_, sender)) = self.waits.remove(call_id) {
            sender.send_replace(true);
        }
    }

    /// Wait for `call_id`'s ACK, at most `within`. At once when no 2xx is
    /// waiting for one.
    pub async fn wait(&self, call_id: &CallId, within: Duration) {
        let Some(mut receiver) = self.waits.get(call_id).map(|s| s.subscribe()) else {
            return;
        };
        let _ = tokio::time::timeout(within, receiver.wait_for(|acked| *acked)).await;
        self.waits.remove(call_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[tokio::test]
    async fn a_bye_waits_for_the_ack() {
        let waits = Arc::new(AckWaits::new());
        let call_id = CallId::new();
        waits.expect(call_id);
        let waiting = {
            let waits = Arc::clone(&waits);
            tokio::spawn(async move { waits.wait(&call_id, Duration::from_secs(5)).await })
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!waiting.is_finished(), "the BYE did not wait for the ACK");
        waits.acked(&call_id);
        tokio::time::timeout(Duration::from_secs(1), waiting)
            .await
            .expect("the ACK did not release the BYE")
            .unwrap();
    }

    #[tokio::test]
    async fn nothing_to_wait_for_once_acked_or_never_expected() {
        let waits = AckWaits::new();
        let call_id = CallId::new();
        waits.wait(&call_id, Duration::from_secs(5)).await;
        waits.expect(call_id);
        waits.acked(&call_id);
        tokio::time::timeout(
            Duration::from_millis(100),
            waits.wait(&call_id, Duration::from_secs(5)),
        )
        .await
        .expect("an ACKed 2xx still held the BYE");
    }
}
