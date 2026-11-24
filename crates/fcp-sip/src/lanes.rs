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
//!
//! A response waits only for the responses before it in its call (a 180
//! before its 200), never for a request: a request's handling can be
//! waiting on that very response. A room's floor INFO relayed to the
//! phone waited for the dialog the phone's re-INVITE held while it waited
//! for the room's 200, and the 200 waited behind the INFO, so every share
//! and stop took the full bound and the room resent its 200 for an ACK
//! that came late. A request still waits for every message before it,
//! responses included, so a BYE is not handled before the 200 it follows.

use bytes::Bytes;
use dashmap::DashMap;
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
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

/// Set once a message's handling is done (or its task gone).
type Done = watch::Receiver<bool>;

struct Inner {
    /// By Call-ID: the last request dispatched (its number) and the signal
    /// for its end, which the next request waits on.
    requests: DashMap<String, (u64, Done)>,
    /// By Call-ID: the last response dispatched, which every later message
    /// of the call waits on.
    responses: DashMap<String, (u64, Done)>,
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
                requests: DashMap::new(),
                responses: DashMap::new(),
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
        let response = payload.starts_with(b"SIP/2.0 ");
        let seq = self.inner.next.fetch_add(1, Ordering::Relaxed);
        let (done, after_this) = watch::channel(false);
        // What this message waits for: the call's last response, and for a
        // request the call's last request as well.
        let mut before: Vec<Done> = Vec::with_capacity(2);
        if response {
            if let Some((_, last)) = self
                .inner
                .responses
                .insert(call_id.clone(), (seq, after_this))
            {
                before.push(last);
            }
        } else {
            if let Some(last) = self.inner.responses.get(&call_id) {
                before.push(last.1.clone());
            }
            if let Some((_, last)) = self
                .inner
                .requests
                .insert(call_id.clone(), (seq, after_this))
            {
                before.push(last);
            }
        }
        let inner = Arc::clone(&self.inner);
        tokio::spawn(async move {
            if !before.is_empty() {
                let all = futures::future::join_all(before.iter_mut().map(|rx| async move {
                    // An error is the message before having finished
                    // without a word (its task gone): nothing to wait for.
                    let _ = rx.wait_for(|done| *done).await;
                }));
                if tokio::time::timeout(inner.overtake_after, all)
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
            let _ = done.send(true);
            let lane = if response {
                &inner.responses
            } else {
                &inner.requests
            };
            lane.remove_if(&call_id, |_, (tail, _)| *tail == seq);
        });
    }

    /// Calls with a message being handled.
    pub fn len(&self) -> usize {
        let mut calls: std::collections::HashSet<String> = self
            .inner
            .requests
            .iter()
            .map(|e| e.key().clone())
            .collect();
        calls.extend(self.inner.responses.iter().map(|e| e.key().clone()));
        calls.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.requests.is_empty() && self.inner.responses.is_empty()
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
    use tokio::sync::oneshot;

    fn response(call_id: &str, what: &str) -> Bytes {
        Bytes::from(format!(
            "SIP/2.0 200 OK\r\nVia: SIP/2.0/UDP x;branch=z9hG4bK{what}\r\nCall-ID: {call_id}\r\nCSeq: 2 INVITE\r\n\r\n{what}"
        ))
    }

    /// A request whose handling waits on a response of its own call (a
    /// floor INFO relayed while the re-INVITE it waits on is out) does not
    /// hold that response: the response is taken at once, not after the
    /// bound.
    #[tokio::test]
    async fn a_response_is_not_held_behind_a_request_waiting_on_it() {
        let lanes = CallLanes::new(Duration::from_secs(5));
        let (answer, answered) = oneshot::channel::<()>();
        let (handled, mut done) = tokio::sync::mpsc::unbounded_channel();
        let info = handled.clone();
        lanes.dispatch(&message("one@h", "info"), async move {
            let _ = answered.await;
            let _ = info.send("info");
        });
        lanes.dispatch(&response("one@h", "200"), async move {
            let _ = answer.send(());
            let _ = handled.send("200");
        });
        let started = std::time::Instant::now();
        assert_eq!(done.recv().await, Some("200"));
        assert_eq!(done.recv().await, Some("info"));
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "held for {:?}",
            started.elapsed()
        );
    }

    /// A request still waits for a response before it in its call: a BYE
    /// is not handled ahead of the 200 it follows.
    #[tokio::test]
    async fn a_request_waits_for_the_response_before_it() {
        let lanes = CallLanes::new(Duration::from_secs(5));
        let order = Arc::new(Mutex::new(Vec::new()));
        let (all_done, mut finished) = tokio::sync::mpsc::unbounded_channel();
        for (payload, what, takes) in [
            (response("one@h", "200"), "200", 100),
            (message("one@h", "bye"), "bye", 0),
        ] {
            let order = Arc::clone(&order);
            let all_done = all_done.clone();
            lanes.dispatch(&payload, async move {
                tokio::time::sleep(Duration::from_millis(takes)).await;
                order.lock().unwrap().push(what);
                let _ = all_done.send(());
            });
        }
        for _ in 0..2 {
            finished.recv().await;
        }
        assert_eq!(*order.lock().unwrap(), ["200", "bye"]);
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(lanes.is_empty(), "{} lanes left", lanes.len());
    }

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
