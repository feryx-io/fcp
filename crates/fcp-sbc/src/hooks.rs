//! SBC call lifecycle hooks.
//!
//! Implements `EventPublisher` to wire B2BUA call events into the SBC's
//! admission controller and runtime metrics. Passed to `CallManager::with_events()`
//! so that `create_call()` / `terminate_call()` automatically update concurrent
//! call counts and outcome counters.

use crate::admission::AdmissionController;
use crate::metrics::SbcRuntimeMetrics;
use anyhow::Result;
use async_trait::async_trait;
use fcp_events::{CallEvent, EventPublisher, PlatformEvent, RegistrationEvent};
use std::sync::Arc;
use tracing::debug;

/// Bridges B2BUA call lifecycle events to SBC admission and metrics.
pub struct SbcCallHooks {
    admission: Arc<AdmissionController>,
    metrics: Arc<SbcRuntimeMetrics>,
}

impl SbcCallHooks {
    pub fn new(admission: Arc<AdmissionController>, metrics: Arc<SbcRuntimeMetrics>) -> Self {
        Self { admission, metrics }
    }
}

#[async_trait]
impl EventPublisher for SbcCallHooks {
    async fn publish(&self, event: PlatformEvent) -> Result<()> {
        if let PlatformEvent::Call(ref call_event) = event {
            match call_event {
                CallEvent::Initiated { .. } => {
                    self.admission.call_started();
                    debug!("SBC: call started, concurrent count incremented");
                }
                CallEvent::Ended { .. } => {
                    self.admission.call_ended();
                    self.metrics.record_call_completed();
                }
                CallEvent::Failed { .. } => {
                    self.admission.call_ended();
                    self.metrics.record_call_rejected();
                }
                _ => {}
            }
        }

        if let PlatformEvent::Registration(ref reg_event) = event {
            match reg_event {
                RegistrationEvent::Registered { .. } => {
                    self.admission.registration_added();
                    debug!("SBC: registration added, concurrent count incremented");
                }
                RegistrationEvent::Unregistered { .. } => {
                    self.admission.registration_removed();
                    debug!("SBC: registration removed, concurrent count decremented");
                }
                RegistrationEvent::Failed { .. } => {}
            }
        }

        Ok(())
    }
}
