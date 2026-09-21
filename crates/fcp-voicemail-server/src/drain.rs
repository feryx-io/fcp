//! What a drain means to a voicemail node: the callers leaving a message,
//! and the messages they have already spoken
//! (`docs/CAPACITY_AND_SURVIVABILITY.md` §6).
//!
//! The sequence itself — refuse, wait, end, or call the whole thing off —
//! is [`fcp_sip::drain`], which every routing binary runs (§13, 2b-2), and
//! the calls are ended by the shared [`fcp_sip::drain::NodeCalls`]. What is
//! here is the one thing a voicemail node has that the others do not: a
//! recording outlives the call it belongs to by a moment. The recorder
//! notices the caller is gone, closes the file, writes the message row and
//! sends the MWI — and a node that stopped the instant it sent the BYE
//! would lose all three. So this target hangs up, then waits for what is
//! being written to be written.

use async_trait::async_trait;
use fcp_sip::drain::{DrainTarget, NodeCalls};
use fcp_voicemail::VoicemailService;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{info, warn};

/// How long the drain gives a message already spoken to reach the
/// database. The recorder looks at the call once a second, so this is that
/// plus the write and the MWI, with room to spare.
const MESSAGE_SETTLE: Duration = Duration::from_secs(10);

/// How often the settle looks at what is left to write.
const TICK: Duration = Duration::from_millis(100);

/// The callers this node is carrying, and their messages.
pub struct CallersLeavingMessages {
    calls: NodeCalls,
    voicemail: Arc<VoicemailService>,
}

impl CallersLeavingMessages {
    pub fn new(calls: NodeCalls, voicemail: Arc<VoicemailService>) -> Self {
        Self { calls, voicemail }
    }
}

#[async_trait]
impl DrainTarget for CallersLeavingMessages {
    async fn carrying(&self) -> usize {
        self.calls.carrying().await
    }

    /// Hang up every caller still leaving a message — each with a `Reason`
    /// and a record that says the node was draining — and then wait for
    /// the messages they had already spoken to be saved.
    async fn end_remaining(&self) -> usize {
        let ended = self.calls.end_remaining().await;

        let deadline = Instant::now() + MESSAGE_SETTLE;
        let mut writing = self.voicemail.messages_being_recorded();
        if writing > 0 {
            info!(messages = writing, "Draining: saving the messages spoken");
        }
        while writing > 0 && Instant::now() < deadline {
            tokio::time::sleep(TICK).await;
            writing = self.voicemail.messages_being_recorded();
        }
        if writing > 0 {
            warn!(
                messages = writing,
                settle_secs = MESSAGE_SETTLE.as_secs(),
                "Some messages were still being saved when the drain ended"
            );
        }
        ended
    }

    async fn advertise(&self, draining: bool) {
        self.calls.advertise(draining).await;
    }
}
