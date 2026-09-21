//! What a drain means to the SBC: the calls it is relaying, the BYE that
//! ends them, and the cluster it tells
//! (`docs/CAPACITY_AND_SURVIVABILITY.md` §6).
//!
//! The sequence itself — refuse, wait, end, or call the whole thing off —
//! is [`fcp_sip::drain`], which every routing binary runs (§13, 2b-1).
//! What is here is only what "what is left" means to an SBC: both legs of
//! every call still up, each told why, each with its record, exactly as
//! the admin API's `terminate_call` does it.

use async_trait::async_trait;
use fcp_sip::drain::{DrainTarget, DRAIN_REASON};
use std::sync::Arc;
use std::time::Duration;
use tracing::warn;

/// The calls this SBC is relaying.
pub struct NodeCalls {
    call_manager: Arc<fcp_b2bua_core::CallManager>,
    uac: Option<Arc<fcp_sip::uac::SipUserAgentClient>>,
    dialog_manager: Arc<sip_dialog::DialogManager>,
    #[cfg(feature = "storage-redis")]
    cluster: Option<Arc<fcp_cluster::ClusterManager>>,
}

impl NodeCalls {
    pub fn new(
        call_manager: Arc<fcp_b2bua_core::CallManager>,
        uac: Option<Arc<fcp_sip::uac::SipUserAgentClient>>,
        dialog_manager: Arc<sip_dialog::DialogManager>,
        #[cfg(feature = "storage-redis")] cluster: Option<Arc<fcp_cluster::ClusterManager>>,
    ) -> Self {
        Self {
            call_manager,
            uac,
            dialog_manager,
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

    /// End both legs of every call still up: the record first, so a node
    /// that is killed between the two still accounts for the call, then a
    /// BYE with the `Reason` that tells the far end this was the platform
    /// rather than a party.
    async fn end_remaining(&self) -> usize {
        let mut ended = 0;
        for call in self.call_manager.all_calls() {
            if call.state.is_terminal() {
                continue;
            }
            let dialogs: Vec<sip_dialog::Dialog> = [
                call.a_leg.dialog.clone(),
                call.b_leg.as_ref().and_then(|leg| leg.dialog.clone()),
            ]
            .into_iter()
            .flatten()
            .collect();
            if let Err(e) = self.call_manager.terminate_call_with(
                &call.id,
                fcp_sip::TerminationReason::system(
                    fcp_sip::SystemCause::NodeDraining,
                    "Node draining",
                ),
            ) {
                warn!(call_id = %call.id, error = %e, "Could not end a call on drain");
                continue;
            }
            ended += 1;
            let Some(uac) = self.uac.clone() else {
                continue;
            };
            for dialog in dialogs {
                let uac = Arc::clone(&uac);
                let dm = Arc::clone(&self.dialog_manager);
                tokio::spawn(async move {
                    if uac.bye_with_reason(&dialog, DRAIN_REASON).await.is_err() {
                        tokio::time::sleep(Duration::from_millis(200)).await;
                        if let Err(e) = uac.bye_with_reason(&dialog, DRAIN_REASON).await {
                            warn!(error = %e, "BYE on drain failed");
                        }
                    }
                    dm.remove(dialog.id());
                });
            }
        }
        ended
    }

    /// Say so in the heartbeat, so nothing places work here while this
    /// node is leaving — whether the drain started at `POST /drain` or at
    /// SIGTERM.
    async fn advertise(&self, _draining: bool) {
        #[cfg(feature = "storage-redis")]
        if let Some(cluster) = &self.cluster {
            if let Err(e) = cluster.set_draining(_draining).await {
                warn!(error = %e, "Could not advertise draining to the cluster");
            }
        }
    }
}

/// A drain over a bare call manager, for the tests that build a
/// [`crate::health::HealthState`] without a node behind it.
#[cfg(test)]
pub fn test_drain(call_manager: Arc<fcp_b2bua_core::CallManager>) -> Arc<fcp_sip::drain::Drain> {
    Arc::new(fcp_sip::drain::Drain::new(
        Arc::new(fcp_sbc_core::admission::AdmissionController::new(
            Default::default(),
        )) as Arc<dyn fcp_sip::admission::NodeAdmission>,
        Arc::new(NodeCalls::new(
            call_manager,
            None,
            Arc::new(sip_dialog::DialogManager::new()),
            #[cfg(feature = "storage-redis")]
            None,
        )) as Arc<dyn fcp_sip::drain::DrainTarget>,
        Arc::new(fcp_events::InMemoryEventBus::new()),
        "test-sbc".to_string(),
        0,
    ))
}
