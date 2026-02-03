//! SBC event hooks (calls + registrations).
//!
//! Implements `EventPublisher` to wire B2BUA call events and registrar events
//! into the SBC's admission controller, runtime metrics, and trunk health
//! monitor. Passed to `CallManager::with_events()` and the registrar so
//! lifecycle changes automatically update concurrent counts and outcome
//! counters.
//!
//! Optionally hooks SIPREC recording: when a `SiprecRecorder` is provided,
//! call events trigger recording session start/stop.

use crate::admission::AdmissionController;
use crate::local_capture::LocalCaptureManager;
use crate::metrics::SbcRuntimeMetrics;
use crate::siprec::SiprecRecorder;
use fcp_sbc_core::trunk_health::TrunkHealthMonitor;
use anyhow::Result;
use async_trait::async_trait;
use fcp_events::{CallEvent, EventPublisher, PlatformEvent, RegistrationEvent};
use std::sync::Arc;
use tracing::debug;

/// Bridges B2BUA + registrar events to SBC admission, metrics, and trunk health.
pub struct SbcEventHooks {
    admission: Arc<AdmissionController>,
    metrics: Arc<SbcRuntimeMetrics>,
    recorder: Option<Arc<SiprecRecorder>>,
    trunk_health: Option<Arc<TrunkHealthMonitor>>,
    local_capture: Option<Arc<LocalCaptureManager>>,
}

impl SbcEventHooks {
    pub fn new(
        admission: Arc<AdmissionController>,
        metrics: Arc<SbcRuntimeMetrics>,
        trunk_health: Option<Arc<TrunkHealthMonitor>>,
        local_capture: Option<Arc<LocalCaptureManager>>,
    ) -> Self {
        Self {
            admission,
            metrics,
            recorder: None,
            trunk_health,
            local_capture,
        }
    }

    pub fn with_recorder(
        admission: Arc<AdmissionController>,
        metrics: Arc<SbcRuntimeMetrics>,
        recorder: Arc<SiprecRecorder>,
        trunk_health: Option<Arc<TrunkHealthMonitor>>,
        local_capture: Option<Arc<LocalCaptureManager>>,
    ) -> Self {
        Self {
            admission,
            metrics,
            recorder: Some(recorder),
            trunk_health,
            local_capture,
        }
    }
}

#[async_trait]
impl EventPublisher for SbcEventHooks {
    async fn publish(&self, event: PlatformEvent) -> Result<()> {
        if let PlatformEvent::Call(ref call_event) = event {
            match call_event {
                CallEvent::Initiated {
                    call_id,
                    from,
                    to,
                    direction,
                    trunk_name,
                    ..
                } => {
                    self.admission.call_started();
                    debug!("SBC: call started, concurrent count incremented");

                    if let Some(ref recorder) = self.recorder {
                        recorder.on_call_initiated(
                            call_id,
                            from,
                            to,
                            direction,
                            trunk_name.as_deref(),
                        );
                    }
                    if let Some(ref capture) = self.local_capture {
                        capture.on_call_initiated(
                            call_id,
                            from,
                            to,
                            direction,
                            trunk_name.as_deref(),
                        );
                    }
                }
                CallEvent::Answered {
                    call_id,
                    a_leg_sdp,
                    b_leg_sdp,
                    ..
                } => {
                    if let Some(ref recorder) = self.recorder {
                        let recorder = Arc::clone(recorder);
                        let call_id_clone = call_id.clone();
                        let a_sdp = a_leg_sdp.clone();
                        let b_sdp = b_leg_sdp.clone();
                        tokio::spawn(async move {
                            recorder
                                .start_recording(
                                    &call_id_clone,
                                    a_sdp.as_deref(),
                                    b_sdp.as_deref(),
                                )
                                .await;
                        });
                    }
                    if let Some(ref capture) = self.local_capture {
                        let capture = Arc::clone(capture);
                        let call_id = call_id.clone();
                        tokio::spawn(async move {
                            capture.on_call_answered(&call_id).await;
                        });
                    }
                }
                CallEvent::Ended {
                    call_id,
                    trunk_name,
                    ..
                } => {
                    self.admission.call_ended();
                    self.metrics.record_call_completed();

                    if let Some(ref health) = self.trunk_health {
                        if let Some(ref name) = trunk_name {
                            health.call_ended(name);
                            health.record_success(name);
                        }
                    }

                    if let Some(ref recorder) = self.recorder {
                        recorder.cleanup_pending(call_id);
                        let recorder = Arc::clone(recorder);
                        let call_id_clone = call_id.clone();
                        tokio::spawn(async move {
                            recorder.stop_recording(&call_id_clone).await;
                        });
                    }
                    if let Some(ref capture) = self.local_capture {
                        let capture = Arc::clone(capture);
                        let call_id = call_id.clone();
                        tokio::spawn(async move {
                            capture.on_call_ended(&call_id).await;
                        });
                    }
                }
                CallEvent::Failed {
                    call_id,
                    sip_code,
                    trunk_name,
                    ..
                } => {
                    self.admission.call_ended();
                    self.metrics.record_call_rejected();

                    if let Some(ref health) = self.trunk_health {
                        if let Some(ref name) = trunk_name {
                            health.call_ended(name);
                            // 5xx and timeouts indicate trunk-side failure;
                            // 4xx responses are client errors, not trunk issues.
                            let is_trunk_failure = sip_code
                                .map(|c| c >= 500 || c == 408)
                                .unwrap_or(true); // no code = likely timeout
                            if is_trunk_failure {
                                health.record_failure(name);
                            }
                        }
                    }

                    if let Some(ref recorder) = self.recorder {
                        recorder.cleanup_pending(call_id);
                        let recorder = Arc::clone(recorder);
                        let call_id_clone = call_id.clone();
                        tokio::spawn(async move {
                            recorder.stop_recording(&call_id_clone).await;
                        });
                    }
                    if let Some(ref capture) = self.local_capture {
                        capture.on_call_failed(call_id);
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
