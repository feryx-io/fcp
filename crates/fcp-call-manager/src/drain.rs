//! What a drain means to this node: the calls it is carrying, the BYE that
//! ends them, and the cluster it tells (`docs/CAPACITY_AND_SURVIVABILITY.md`
//! §6).
//!
//! The sequence itself — refuse, wait, end, or call the whole thing off —
//! is [`fcp_sip::drain`], which every routing binary runs (§13, 2b-1). All
//! that is here is what "what is left" means to a call manager.

use async_trait::async_trait;
use fcp_sip::drain::{DrainTarget, DRAIN_REASON};
use fcp_sip::handler::SipMessageHandler;
use fcp_sip::CallManager;
use std::sync::Arc;
use tracing::warn;

/// The calls this node is carrying.
pub struct NodeCalls {
    handler: Arc<SipMessageHandler>,
    call_manager: Arc<CallManager>,
    #[cfg(feature = "storage-redis")]
    cluster: Option<Arc<fcp_cluster::ClusterManager>>,
}

impl NodeCalls {
    pub fn new(
        handler: Arc<SipMessageHandler>,
        call_manager: Arc<CallManager>,
        #[cfg(feature = "storage-redis")] cluster: Option<Arc<fcp_cluster::ClusterManager>>,
    ) -> Self {
        Self {
            handler,
            call_manager,
            #[cfg(feature = "storage-redis")]
            cluster,
        }
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
    async fn advertise(&self, _draining: bool) {
        #[cfg(feature = "storage-redis")]
        if let Some(cluster) = &self.cluster {
            if let Err(e) = cluster.set_draining(_draining).await {
                warn!(error = %e, "Could not advertise draining to the cluster");
            }
        }
    }
}
