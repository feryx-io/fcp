//! SBC event hooks (calls + registrations).
//!
//! Implements `EventPublisher` to wire B2BUA call events and registrar events
//! into the SBC's admission controller and runtime metrics. Passed to
//! `CallManager::with_events()` and the registrar so lifecycle changes
//! automatically update concurrent counts and outcome counters.
//!
//! Optionally hooks SIPREC recording: when a `SiprecRecorder` is provided,
//! call events trigger recording session start/stop.

use crate::admission::AdmissionController;
use crate::metrics::SbcRuntimeMetrics;
use crate::siprec::SiprecRecorder;
use anyhow::Result;
use async_trait::async_trait;
use fcp_events::{CallEvent, EventPublisher, PlatformEvent, RegistrationEvent};
use std::sync::Arc;
use tracing::debug;

/// Bridges B2BUA + registrar events to SBC admission and metrics.
pub struct SbcEventHooks {
    admission: Arc<AdmissionController>,
    metrics: Arc<SbcRuntimeMetrics>,
    recorder: Option<Arc<SiprecRecorder>>,
}

impl SbcEventHooks {
    pub fn new(admission: Arc<AdmissionController>, metrics: Arc<SbcRuntimeMetrics>) -> Self {
        Self {
            admission,
            metrics,
            recorder: None,
        }
    }

    pub fn with_recorder(
        admission: Arc<AdmissionController>,
        metrics: Arc<SbcRuntimeMetrics>,
        recorder: Arc<SiprecRecorder>,
    ) -> Self {
        Self {
            admission,
            metrics,
            recorder: Some(recorder),
        }
    }
}

#[async_trait]
impl EventPublisher for SbcEventHooks {
    async fn publish(&self, event: PlatformEvent) -> Result<()> {
        if let PlatformEvent::Call(ref call_event) = event {
            match call_event {
                CallEvent::Initiated {
                    call_id, from, to, ..
                } => {
                    self.admission.call_started();
                    debug!("SBC: call started, concurrent count incremented");

                    if let Some(ref recorder) = self.recorder {
                        recorder.on_call_initiated(call_id, from, to);
                    }
                }
                CallEvent::Answered { call_id, .. } => {
                    if let Some(ref recorder) = self.recorder {
                        let recorder = Arc::clone(recorder);
                        let call_id = call_id.clone();
                        tokio::spawn(async move {
                            recorder.start_recording(&call_id).await;
                        });
                    }
                }
                CallEvent::Ended { call_id, .. } => {
                    self.admission.call_ended();
                    self.metrics.record_call_completed();

                    if let Some(ref recorder) = self.recorder {
                        recorder.cleanup_pending(call_id);
                        let recorder = Arc::clone(recorder);
                        let call_id = call_id.clone();
                        tokio::spawn(async move {
                            recorder.stop_recording(&call_id).await;
                        });
                    }
                }
                CallEvent::Failed { call_id, .. } => {
                    self.admission.call_ended();
                    self.metrics.record_call_rejected();

                    if let Some(ref recorder) = self.recorder {
                        recorder.cleanup_pending(call_id);
                        let recorder = Arc::clone(recorder);
                        let call_id = call_id.clone();
                        tokio::spawn(async move {
                            recorder.stop_recording(&call_id).await;
                        });
                    }
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
