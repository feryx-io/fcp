//! Leaving service without dropping what this node is carrying
//! (`docs/CAPACITY_AND_SURVIVABILITY.md` §6).
//!
//! A drain is the same sequence wherever it runs and however it starts —
//! `POST /drain` or SIGTERM: tell the cluster, refuse new work, keep what
//! is already up, and at the deadline end what is left saying why. A drain
//! called off before the deadline (`DELETE /drain`) leaves the node serving
//! again, with everything it was carrying untouched.
//!
//! The sequence lives here so that every routing binary runs the same one
//! (§13, 2b-1). What differs per binary is the last step — what "end what
//! is left" means to it — and that is [`DrainTarget`], which the binary
//! supplies: a BYE on both legs of every call for the call manager, nothing
//! but a shorter registration for the registrar. The one every B2BUA node
//! shares — the call manager, the IVR, the voicemail server — is
//! [`NodeCalls`], which is here rather than in each of them (§13, 2b-2).
//!
//! The HTTP bodies are here too, as status and JSON rather than as routes,
//! so that a binary's own router shape and its own authentication decide
//! where `/drain` hangs while every node still answers the same thing.

use crate::admission::NodeAdmission;
use async_trait::async_trait;
use fcp_events::{EventPublisher, PlatformEvent, SystemEvent};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};
use tracing::{info, warn};

/// What the far end is told when the platform, rather than a party, ends
/// the call (RFC 3326).
pub const DRAIN_REASON: &str = "SIP;cause=200;text=\"node draining\"";

/// How often the wait looks at what is left.
const TICK: Duration = Duration::from_secs(1);

/// What a binary gives the drain: what it is carrying, how to end it, and
/// how to tell the cluster it is leaving.
#[async_trait]
pub trait DrainTarget: Send + Sync {
    /// How much work this node is carrying — calls for the call manager
    /// and the SBC, sessions for the gateway, the registrations still held
    /// for the registrar. A drain waits for this to reach zero.
    async fn carrying(&self) -> usize;

    /// End what is left, each with a reason that says the node was
    /// draining, and each with the record that goes with it. How many were
    /// ended.
    async fn end_remaining(&self) -> usize;

    /// Tell the cluster this node is leaving, or serving again. A node
    /// that is not in a cluster has nothing to tell.
    async fn advertise(&self, _draining: bool) {}
}

/// How a binary tells its cluster that this node is leaving, when it is in
/// one. `fcp-cluster` is not a dependency of this crate, so what is passed
/// is an adapter over `ClusterManager` — `fcp_sbc_core::ClusterAnnouncer`,
/// which every binary that has a cluster manager uses.
#[async_trait]
pub trait DrainAnnouncer: Send + Sync {
    /// Say so in the heartbeat, so nothing places work here while this node
    /// is leaving.
    async fn set_draining(&self, draining: bool);
}

/// The calls a B2BUA node is carrying, ended with a BYE on both legs.
///
/// What the call manager, the IVR and the voicemail server all mean by
/// "what is left": every call the node still has, hung up with an RFC 3326
/// `Reason` that says the platform ended it, and a record that says the
/// node was draining. The SBC keeps its own target — it relays calls it
/// has no `SipMessageHandler` for.
pub struct NodeCalls {
    handler: Arc<crate::handler::SipMessageHandler>,
    call_manager: Arc<crate::CallManager>,
    announcer: Option<Arc<dyn DrainAnnouncer>>,
}

impl NodeCalls {
    pub fn new(
        handler: Arc<crate::handler::SipMessageHandler>,
        call_manager: Arc<crate::CallManager>,
    ) -> Self {
        Self {
            handler,
            call_manager,
            announcer: None,
        }
    }

    /// The cluster this node is in, when it is in one.
    pub fn with_announcer(mut self, announcer: Option<Arc<dyn DrainAnnouncer>>) -> Self {
        self.announcer = announcer;
        self
    }
}

#[async_trait]
impl DrainTarget for NodeCalls {
    async fn carrying(&self) -> usize {
        self.call_manager.active_call_count()
    }

    /// BYE both legs of every call still up, each with a `Reason` and a
    /// record that says the node was draining.
    async fn end_remaining(&self) -> usize {
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

    /// Say so in the heartbeat, so that placement elsewhere — conference
    /// rooms, queue dial-out — stops choosing this node within an
    /// interval. A drain started at `POST /drain` says it as much as one
    /// started at SIGTERM.
    async fn advertise(&self, draining: bool) {
        if let Some(announcer) = &self.announcer {
            announcer.set_draining(draining).await;
        }
    }
}

/// The node's drain, and what it needs to run one.
pub struct Drain {
    admission: Arc<dyn NodeAdmission>,
    target: Arc<dyn DrainTarget>,
    events: Arc<dyn EventPublisher>,
    node_id: String,
    wait: Duration,
    /// Bumped by every start and every stop, so a wait that is no longer
    /// the current drain gives up quietly.
    generation: AtomicU64,
}

impl Drain {
    pub fn new(
        admission: Arc<dyn NodeAdmission>,
        target: Arc<dyn DrainTarget>,
        events: Arc<dyn EventPublisher>,
        node_id: String,
        wait_secs: u64,
    ) -> Self {
        Self {
            admission,
            target,
            events,
            node_id,
            wait: Duration::from_secs(wait_secs),
            generation: AtomicU64::new(0),
        }
    }

    /// Whether this node is leaving service.
    pub fn is_draining(&self) -> bool {
        self.admission.draining()
    }

    /// How long a drain waits for what this node is carrying.
    pub fn wait_secs(&self) -> u64 {
        self.wait.as_secs()
    }

    /// What this node is carrying.
    pub async fn carrying(&self) -> usize {
        self.target.carrying().await
    }

    /// Start refusing new work. Returns the generation this drain runs
    /// under; the wait stops as soon as that is no longer current.
    ///
    /// Announcing comes first so that whatever places work here — another
    /// node, a load balancer reading `/ready` — stops before this node
    /// does.
    pub async fn start(&self) -> u64 {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.admission.set_draining(true);
        self.target.advertise(true).await;
        let calls = self.carrying().await;
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

    /// Serve again: new work is taken, and what this node is carrying —
    /// which a drain never touched — carries on.
    pub async fn stop(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.admission.set_draining(false);
        self.target.advertise(false).await;
        info!("Drain called off: this node is serving again");
    }

    /// Wait for what this node is carrying to end, and end whatever is
    /// left at the deadline. Returns how many were ended, or `None` if the
    /// drain was called off while waiting.
    pub async fn wait_then_end(&self, generation: u64) -> Option<usize> {
        let started = Instant::now();
        let deadline = started + self.wait;
        while self.carrying().await > 0 && Instant::now() < deadline {
            tokio::time::sleep(TICK).await;
            if !self.is_current(generation) {
                return None;
            }
            let left = self.carrying().await;
            if left > 0 && started.elapsed().as_secs().is_multiple_of(10) {
                info!(calls = left, "Draining: waiting for calls to end");
            }
        }
        if !self.is_current(generation) {
            return None;
        }

        let ended = self.target.end_remaining().await;
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
    /// container runtime's grace period forces anyway. What is ended still
    /// gets its BYE and its record — the point of draining at all.
    pub async fn end_now(&self) -> usize {
        let ended = self.target.end_remaining().await;
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

    fn is_current(&self, generation: u64) -> bool {
        self.generation.load(Ordering::SeqCst) == generation && self.is_draining()
    }

    async fn publish(&self, event: SystemEvent) {
        if let Err(e) = self.events.publish(PlatformEvent::System(event)).await {
            warn!(error = %e, "Could not publish the drain event");
        }
    }
}

// ── The routes, as status and body ─────────────────────────────────────
//
// A binary wraps each of these in its own handler, so `/ready` and
// `/drain` answer the same thing on every node while each router keeps its
// own shape and its own authentication.

/// `503` while this node is draining, so a load balancer stops choosing it
/// before it stops taking calls. `/health` keeps answering `200`
/// throughout, so a liveness probe does not kill the process mid-drain.
pub async fn ready_body(drain: &Drain) -> (u16, serde_json::Value) {
    let draining = drain.is_draining();
    let body = serde_json::json!({
        "ready": !draining,
        "draining": draining,
        "active_calls": drain.carrying().await,
    });
    (if draining { 503 } else { 200 }, body)
}

/// `POST /drain`: this node takes no new calls, keeps the ones it has
/// until `drain_wait_secs`, and then ends what is left with a BYE that
/// says why. The node keeps running — a drain is not a shutdown.
///
/// The wait runs in its own task, so the request answers at once.
pub async fn start_body(drain: &Arc<Drain>) -> (u16, serde_json::Value) {
    if drain.is_draining() {
        return (
            200,
            serde_json::json!({
                "status": "draining",
                "already": true,
                "active_calls": drain.carrying().await,
            }),
        );
    }
    let generation = drain.start().await;
    let calls = drain.carrying().await;
    let waiting = Arc::clone(drain);
    tokio::spawn(async move {
        waiting.wait_then_end(generation).await;
    });
    (
        200,
        serde_json::json!({
            "status": "draining",
            "active_calls": calls,
            "wait_secs": drain.wait_secs(),
        }),
    )
}

/// `DELETE /drain`: the node serves again, and what it was carrying —
/// which the drain never touched — carries on.
pub async fn stop_body(drain: &Drain) -> (u16, serde_json::Value) {
    drain.stop().await;
    (
        200,
        serde_json::json!({
            "status": "serving",
            "active_calls": drain.carrying().await,
        }),
    )
}

/// The shutdown sequence, once a node has decided to stop: start the drain
/// (or take over one that `POST /drain` began), wait for what it carries,
/// and cut the wait short if a second signal says the operator will not
/// wait. Returns when the node is empty, or when what was left has been
/// ended.
///
/// The caller stops its listeners afterwards, as it always did.
pub async fn drain_for_shutdown(drain: &Arc<Drain>) {
    let generation = drain.start().await;
    let second = async {
        let mut sigterm =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(s) => s,
                Err(e) => {
                    warn!(error = %e, "Could not listen for a second SIGTERM while draining");
                    return;
                }
            };
        tokio::select! {
            _ = sigterm.recv() => warn!("Second SIGTERM while draining; ending the calls still up"),
            _ = tokio::signal::ctrl_c() => {
                warn!("Second SIGINT while draining; ending the calls still up")
            }
        }
    };
    tokio::select! {
        _ = drain.wait_then_end(generation) => {}
        _ = second => {
            drain.end_now().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admission::Refusal;
    use fcp_core::TenantId;
    use std::net::IpAddr;
    use std::sync::atomic::{AtomicBool, AtomicUsize};

    /// An admission that does nothing but remember whether it is draining.
    #[derive(Default)]
    struct Admission {
        draining: AtomicBool,
    }

    impl NodeAdmission for Admission {
        fn admit(&self, _call_id: &str, _tenant: &TenantId) -> Result<(), Refusal> {
            if self.draining.load(Ordering::SeqCst) {
                return Err(Refusal::Draining);
            }
            Ok(())
        }
        fn ended(&self, _call_id: &str) {}
        fn admit_register(&self, _source: IpAddr, _aor: &str) -> Result<(), Refusal> {
            Ok(())
        }
        fn draining(&self) -> bool {
            self.draining.load(Ordering::SeqCst)
        }
        fn set_draining(&self, draining: bool) {
            self.draining.store(draining, Ordering::SeqCst);
        }
    }

    /// A node carrying `calls`, which `end_remaining` empties.
    #[derive(Default)]
    struct Node {
        calls: AtomicUsize,
        ended: AtomicUsize,
        advertised: AtomicUsize,
    }

    #[async_trait]
    impl DrainTarget for Node {
        async fn carrying(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
        async fn end_remaining(&self) -> usize {
            let left = self.calls.swap(0, Ordering::SeqCst);
            self.ended.fetch_add(left, Ordering::SeqCst);
            left
        }
        async fn advertise(&self, _draining: bool) {
            self.advertised.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn drain(node: Arc<Node>, wait_secs: u64) -> (Arc<Drain>, Arc<Admission>) {
        let admission = Arc::new(Admission::default());
        let drain = Arc::new(Drain::new(
            Arc::clone(&admission) as Arc<dyn NodeAdmission>,
            node as Arc<dyn DrainTarget>,
            Arc::new(fcp_events::InMemoryEventBus::new()),
            "node-1".to_string(),
            wait_secs,
        ));
        (drain, admission)
    }

    #[tokio::test(start_paused = true)]
    async fn a_drain_refuses_new_calls_and_ends_what_is_left_at_the_deadline() {
        let node = Arc::new(Node::default());
        node.calls.store(2, Ordering::SeqCst);
        let (drain, admission) = drain(Arc::clone(&node), 30);

        let generation = drain.start().await;
        assert!(drain.is_draining(), "the node is leaving service");
        assert_eq!(
            admission.admit("call-1", &TenantId::new()),
            Err(Refusal::Draining),
            "new calls are refused"
        );
        assert_eq!(node.advertised.load(Ordering::SeqCst), 1, "cluster told");
        assert_eq!(
            node.carrying().await,
            2,
            "the calls already up are untouched"
        );

        assert_eq!(drain.wait_then_end(generation).await, Some(2));
        assert_eq!(node.ended.load(Ordering::SeqCst), 2, "both got their BYE");
    }

    #[tokio::test(start_paused = true)]
    async fn a_drain_that_empties_ends_nothing() {
        let node = Arc::new(Node::default());
        node.calls.store(1, Ordering::SeqCst);
        let (drain, _) = drain(Arc::clone(&node), 30);
        let generation = drain.start().await;

        let waiting = Arc::clone(&drain);
        let wait = tokio::spawn(async move { waiting.wait_then_end(generation).await });
        tokio::time::sleep(Duration::from_secs(2)).await;
        node.calls.store(0, Ordering::SeqCst);

        assert_eq!(wait.await.unwrap(), Some(0), "nothing was left to end");
        assert_eq!(node.ended.load(Ordering::SeqCst), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn a_drain_called_off_leaves_the_calls_alone() {
        let node = Arc::new(Node::default());
        node.calls.store(3, Ordering::SeqCst);
        let (drain, admission) = drain(Arc::clone(&node), 30);
        let generation = drain.start().await;

        let waiting = Arc::clone(&drain);
        let wait = tokio::spawn(async move { waiting.wait_then_end(generation).await });
        tokio::time::sleep(Duration::from_secs(2)).await;
        drain.stop().await;

        assert_eq!(wait.await.unwrap(), None, "the wait gave up quietly");
        assert!(!drain.is_draining());
        assert!(admission.admit("call-2", &TenantId::new()).is_ok());
        assert_eq!(node.carrying().await, 3, "still carrying every call");
        assert_eq!(node.ended.load(Ordering::SeqCst), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn ending_now_does_not_wait_for_the_deadline() {
        let node = Arc::new(Node::default());
        node.calls.store(4, Ordering::SeqCst);
        let (drain, _) = drain(Arc::clone(&node), 3600);
        drain.start().await;

        assert_eq!(drain.end_now().await, 4);
        assert_eq!(node.ended.load(Ordering::SeqCst), 4);
    }

    #[tokio::test(start_paused = true)]
    async fn the_routes_say_the_same_thing_the_node_does() {
        let node = Arc::new(Node::default());
        node.calls.store(1, Ordering::SeqCst);
        let (drain, _) = drain(Arc::clone(&node), 30);

        let (code, body) = ready_body(&drain).await;
        assert_eq!(code, 200);
        assert_eq!(body["ready"], true);

        let (code, body) = start_body(&drain).await;
        assert_eq!(code, 200);
        assert_eq!(body["status"], "draining");
        assert_eq!(body["active_calls"], 1);
        assert_eq!(body["wait_secs"], 30);

        let (code, body) = ready_body(&drain).await;
        assert_eq!(code, 503, "a load balancer stops choosing this node");
        assert_eq!(body["draining"], true);

        let (code, body) = start_body(&drain).await;
        assert_eq!(code, 200);
        assert_eq!(body["already"], true, "draining twice is not an error");

        let (code, body) = stop_body(&drain).await;
        assert_eq!(code, 200);
        assert_eq!(body["status"], "serving");
        assert_eq!(ready_body(&drain).await.0, 200);
        assert_eq!(node.ended.load(Ordering::SeqCst), 0, "no call was ended");
    }
}
