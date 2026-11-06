//! One call's inbound messages handled in the order they arrived (#313).
//!
//! Every inbound packet is handled on a task of its own, so that a message
//! whose handling waits on the network (a re-INVITE relayed to the other
//! leg, a REGISTER against a slow store) never holds up the next. Two
//! messages of one dialog that arrive back to back could then be handled in
//! either order: a callee's 180 and 200 were taken 200 first and the 180
//! dropped, and an ACK and a BYE, INFO digits or consecutive NOTIFYs could
//! be swapped the same way.
//!
//! A lane per Call-ID puts them back in order: a message's handling starts
//! once the one before it in its call has finished. Waiting for that is
//! bounded ([`OVERTAKE_AFTER`]): a message whose handling waits on the far
//! end, such as a re-INVITE, is overtaken once it has run that long, as
//! every message was before, so a CANCEL or a BYE behind it is never held
//! for the far end's answer. The bound also keeps the node's loopback legs,
//! where one Call-ID is both a UAC and a UAS dialog of the same node, from
//! waiting on themselves for longer than that.

use bytes::Bytes;
use dashmap::DashMap;
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;
use tracing::info;

/// How long a message waits for the one before it in its call before it
/// is handled anyway.
pub const OVERTAKE_AFTER: Duration = Duration::from_millis(500);

/// The calls with a message being handled, each with a signal for when its
/// last message is done.
#[derive(Clone)]
pub struct CallLanes {
    inner: Arc<Inner>,
}

struct Inner {
    /// By Call-ID: the last message dispatched (its number) and the signal
    /// the next message of the call waits on.
    tails: DashMap<String, (u64, oneshot::Receiver<()>)>,
    next: AtomicU64,
    overtake_after: Duration,
}

impl Default for CallLanes {
    fn default() -> Self {
        Self::new(OVERTAKE_AFTER)
    }
}

impl CallLanes {
    pub fn new(overtake_after: Duration) -> Self {
        Self {
            inner: Arc::new(Inner {
                tails: DashMap::new(),
                next: AtomicU64::new(0),
                overtake_after,
            }),
        }
    }

    /// Handle `payload` on a task of its own, once the message before it
    /// in its call is done. Called in arrival order: the order of these
    /// calls is the order the messages are handled in. A message with no
    /// Call-ID (a keepalive, junk) waits for nothing.
    pub fn dispatch<F>(&self, payload: &Bytes, handle: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let Some(call_id) = call_id_of(payload) else {
            tokio::spawn(handle);
            return;
        };
        let seq = self.inner.next.fetch_add(1, Ordering::Relaxed);
        let (done, after_this) = oneshot::channel();
        let before = self
            .inner
            .tails
            .insert(call_id.clone(), (seq, after_this))
            .map(|(_, before)| before);
        let inner = Arc::clone(&self.inner);
        tokio::spawn(async move {
            if let Some(before) = before {
                // An error is the message before having finished without a
                // word (its task gone): nothing left to wait for.
                if tokio::time::timeout(inner.overtake_after, before)
                    .await
                    .is_err()
                {
                    info!(
                        call_id = %call_id,
                        waited_ms = inner.overtake_after.as_millis() as u64,
                        "The call's previous message is still being handled; going ahead"
                    );
                }
            }
            handle.await;
            let _ = done.send(());
            inner.tails.remove_if(&call_id, |_, (tail, _)| *tail == seq);
        });
    }

    /// Calls with a message being handled.
    pub fn len(&self) -> usize {
        self.inner.tails.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.tails.is_empty()
    }
}

/// A SIP message's Call-ID, read from its header section without parsing
/// the message (the handler parses it again in full). The compact form `i`
/// is a Call-ID too (RFC 3261 §7.3.3).
pub fn call_id_of(payload: &[u8]) -> Option<String> {
    let end = payload
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .unwrap_or(payload.len());
    let head = std::str::from_utf8(&payload[..end]).ok()?;
    head.lines().skip(1).find_map(|line| {
        let (name, value) = line.split_once(':')?;
        let name = name.trim();
        (name.eq_ignore_ascii_case("call-id") || name.eq_ignore_ascii_case("i"))
            .then(|| value.trim())
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn message(call_id: &str, what: &str) -> Bytes {
        Bytes::from(format!(
            "INFO sip:a@b SIP/2.0\r\nVia: SIP/2.0/UDP x;branch=z9hG4bK{what}\r\nCall-ID: {call_id}\r\nCSeq: 1 INFO\r\n\r\n{what}"
        ))
    }

    #[test]
    fn the_call_id_is_read_in_either_form() {
        assert_eq!(
            call_id_of(b"BYE sip:a@b SIP/2.0\r\nVia: x\r\ncall-id:  abc@h \r\n\r\n").as_deref(),
            Some("abc@h")
        );
        assert_eq!(
            call_id_of(b"SIP/2.0 200 OK\r\ni: def@h\r\n\r\n").as_deref(),
            Some("def@h")
        );
        // Only the headers: a body naming a Call-ID is not one.
        assert_eq!(
            call_id_of(b"INFO sip:a@b SIP/2.0\r\nVia: x\r\n\r\nCall-ID: no"),
            None
        );
        assert_eq!(call_id_of(b"\r\n\r\n"), None);
        assert_eq!(call_id_of(b"\xff\xfe"), None);
    }

    /// The first message of a call takes longer to handle than the second:
    /// the second still waits for it, and another call waits for neither.
    #[tokio::test]
    async fn a_calls_messages_are_handled_in_the_order_they_arrived() {
        let lanes = CallLanes::new(Duration::from_secs(5));
        let order = Arc::new(Mutex::new(Vec::new()));
        let (all_done, mut finished) = tokio::sync::mpsc::unbounded_channel();
        for (call_id, what, takes) in [
            ("one@h", "1st", 100),
            ("one@h", "2nd", 0),
            ("two@h", "other", 0),
            ("one@h", "3rd", 0),
        ] {
            let order = Arc::clone(&order);
            let all_done = all_done.clone();
            lanes.dispatch(&message(call_id, what), async move {
                tokio::time::sleep(Duration::from_millis(takes)).await;
                order.lock().unwrap().push(what);
                let _ = all_done.send(());
            });
        }
        for _ in 0..4 {
            finished.recv().await;
        }
        assert_eq!(*order.lock().unwrap(), ["other", "1st", "2nd", "3rd"]);
        // Every lane is let go once its messages are done.
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(lanes.is_empty(), "{} lanes left", lanes.len());
    }

    /// A message still being handled past the bound is overtaken: the
    /// next message of its call is not held for the far end's answer.
    #[tokio::test]
    async fn a_message_waiting_on_the_far_end_is_overtaken() {
        let lanes = CallLanes::new(Duration::from_millis(50));
        let (far_end, answered) = oneshot::channel::<()>();
        let (handled, mut done) = tokio::sync::mpsc::unbounded_channel();
        let first = handled.clone();
        lanes.dispatch(&message("one@h", "reinvite"), async move {
            let _ = answered.await;
            let _ = first.send("reinvite");
        });
        lanes.dispatch(&message("one@h", "bye"), async move {
            let _ = handled.send("bye");
        });
        let next = tokio::time::timeout(Duration::from_secs(2), done.recv())
            .await
            .expect("the BYE was held behind the re-INVITE");
        assert_eq!(next, Some("bye"));
        let _ = far_end.send(());
        assert_eq!(done.recv().await, Some("reinvite"));
    }
}
