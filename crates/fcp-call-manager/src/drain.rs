//! Leaving service without dropping calls
//! (`docs/CAPACITY_AND_SURVIVABILITY.md` §6).
//!
//! A drain is the same sequence however it starts — `POST /drain` or
//! SIGTERM: tell the cluster, refuse new calls, keep the ones already up,
//! and at the deadline end what is left with a BYE that says why. A drain
//! called off before the deadline (`DELETE /drain`) leaves the node
//! serving again, with every call it was carrying untouched.

use fcp_events::{EventPublisher, PlatformEvent, SystemEvent};
use fcp_sip::handler::SipMessageHandler;
use fcp_sip::CallManager;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};
use tracing::{info, warn};

/// What the far end is told when the platform, rather than a party, ends
/// the call (RFC 3326).
pub const DRAIN_REASON: &str = "SIP;cause=200;text=\"node draining\"";

/// How often the wait looks at what is left.
const TICK: Duration = Duration::from_secs(1);

/// The node's drain, and what it needs to run one.
pub struct Drain {
    admission: Arc<fcp_sbc_core::AdmissionController>,
    handler: Arc<SipMessageHandler>,
    call_manager: Arc<CallManager>,
    events: Arc<dyn EventPublisher>,
    node_id: String,
    wait: Duration,
    /// Bumped by every start and every stop, so a wait that is no longer
    /// the current drain gives up quietly.
    generation: AtomicU64,
}

impl Drain {
    pub fn new(
        admission: Arc<fcp_sbc_core::AdmissionController>,
        handler: Arc<SipMessageHandler>,
        call_manager: Arc<CallManager>,
        events: Arc<dyn EventPublisher>,
        node_id: String,
        wait_secs: u64,
    ) -> Self {
        Self {
            admission,
            handler,
            call_manager,
            events,
            node_id,
            wait: Duration::from_secs(wait_secs),
            generation: AtomicU64::new(0),
        }
    }

    /// Whether this node is leaving service.
    pub fn is_draining(&self) -> bool {
        self.admission.is_draining()
    }

    /// How long a drain waits for the calls it is carrying.
    pub fn wait_secs(&self) -> u64 {
        self.wait.as_secs()
    }

    /// Calls this node is carrying.
    fn calls(&self) -> usize {
        self.call_manager.active_call_count()
    }

    /// Start refusing new calls. Returns the generation this drain runs
    /// under; the wait stops as soon as that is no longer current.
    ///
    /// Announcing comes first so that whatever places calls here — another
    /// node, a load balancer reading `/ready` — stops before this node
    /// does.
    pub async fn start(&self) -> u64 {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.admission.set_draining(true);
        let calls = self.calls();
        info!(
            calls,
            wait_secs = self.wait.as_secs(),
            "Draining: this node takes no new calls"
        );
        self.publish(SystemEvent::NodeDraining {
            node_id: self.node_id.clone(),
            calls,
            wait_secs: self.wait.as_secs(),
            timestamp: SystemTime::now(),
        })
        .await;
        generation
    }

    /// Serve again: new calls are taken, and the calls this node is
    /// carrying — which a drain never touched — carry on.
    pub fn stop(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.admission.set_draining(false);
        info!("Drain called off: this node is serving again");
    }

    /// Wait for the calls this node is carrying to end, and hang up
    /// whatever is left at the deadline. Returns how many were hung up,
    /// or `None` if the drain was called off while waiting.
    pub async fn wait_then_end(&self, generation: u64) -> Option<usize> {
        let started = Instant::now();
        let deadline = started + self.wait;
        while self.calls() > 0 && Instant::now() < deadline {
            tokio::time::sleep(TICK).await;
            if !self.is_current(generation) {
                return None;
            }
            let left = self.calls();
            if left > 0 && started.elapsed().as_secs().is_multiple_of(10) {
                info!(calls = left, "Draining: waiting for calls to end");
            }
        }
        if !self.is_current(generation) {
            return None;
        }

        let ended = self.end_remaining();
        let waited_secs = started.elapsed().as_secs();
        if ended == 0 {
            info!(waited_secs, "Drained: every call had ended");
        } else {
            warn!(
                calls_ended = ended,
                waited_secs, "Drain wait elapsed; the calls still up were hung up"
            );
        }
        self.publish(SystemEvent::NodeDrained {
            node_id: self.node_id.clone(),
            calls_ended: ended,
            waited_secs,
            timestamp: SystemTime::now(),
        })
        .await;
        Some(ended)
    }

    /// End what is left now, without waiting: what a second SIGTERM asks
    /// for when an operator will not wait out the drain, and what a
    /// container runtime's grace period forces anyway. The calls still
    /// get their BYE and their record — the point of draining at all.
    pub async fn end_now(&self) -> usize {
        let ended = self.end_remaining();
        if ended > 0 {
            warn!(
                calls_ended = ended,
                "Drain cut short; the calls still up were hung up"
            );
        }
        self.publish(SystemEvent::NodeDrained {
            node_id: self.node_id.clone(),
            calls_ended: ended,
            waited_secs: 0,
            timestamp: SystemTime::now(),
        })
        .await;
        ended
    }

    /// BYE both legs of every call still up, each with a `Reason` and a
    /// record that says the node was draining.
    fn end_remaining(&self) -> usize {
        let mut ended = 0;
        for call in self.call_manager.all_calls() {
            if call.state.is_terminal() {
                continue;
            }
            match self.handler.hangup_call_by_id_saying(
                &call.id,
                fcp_cdr::TerminationReason::system(
                    fcp_cdr::SystemCause::NodeDraining,
                    "Node draining",
                ),
                Some(DRAIN_REASON),
            ) {
                Ok(()) => ended += 1,
                Err(e) => {
                    warn!(call_id = %call.id, error = %e, "Could not hang up a call on drain")
                }
            }
        }
        ended
    }

    fn is_current(&self, generation: u64) -> bool {
        self.generation.load(Ordering::SeqCst) == generation && self.is_draining()
    }

    async fn publish(&self, event: SystemEvent) {
        if let Err(e) = self.events.publish(PlatformEvent::System(event)).await {
            warn!(error = %e, "Could not publish the drain event");
        }
    }
}
