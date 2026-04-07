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
use anyhow::Result;
use async_trait::async_trait;
use dashmap::DashMap;
use fcp_events::{CallEvent, EventPublisher, PlatformEvent, RegistrationEvent};
use fcp_sbc_core::trunk_health::TrunkHealthMonitor;
use fcp_sbc_core::trunk_quality::TrunkQualityTracker;
use std::sync::Arc;
use std::time::SystemTime;
use tracing::debug;

/// Per-call tracking state for quality metrics (PDD computation).
struct CallQualityState {
    trunk_name: String,
    initiated_at: SystemTime,
}

/// Bridges B2BUA + registrar events to SBC admission, metrics, and trunk health.
pub struct SbcEventHooks {
    admission: Arc<AdmissionController>,
    metrics: Arc<SbcRuntimeMetrics>,
    recorder: Option<Arc<SiprecRecorder>>,
    trunk_health: Option<Arc<TrunkHealthMonitor>>,
    local_capture: Option<Arc<LocalCaptureManager>>,
    quality: Option<Arc<TrunkQualityTracker>>,
    /// Tracks per-call (trunk_name, initiated_time) for PDD computation.
    call_quality_state: DashMap<String, CallQualityState>,
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
            quality: None,
            call_quality_state: DashMap::new(),
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
            quality: None,
            call_quality_state: DashMap::new(),
        }
    }

    /// Set an optional quality tracker for per-trunk metrics (ASR/ACD/PDD).
    pub fn with_quality(mut self, quality: Arc<TrunkQualityTracker>) -> Self {
        self.quality = Some(quality);
        self
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
                    timestamp,
                    ..
                } => {
                    self.admission.call_started();
                    debug!("SBC: call started, concurrent count incremented");

                    if let Some(ref quality) = self.quality {
                        if let Some(ref name) = trunk_name {
                            quality.record_call_attempt(name);
                            // Store per-call state for PDD computation at answer time
                            self.call_quality_state.insert(
                                call_id.to_string(),
                                CallQualityState {
                                    trunk_name: name.clone(),
                                    initiated_at: *timestamp,
                                },
                            );
                        }
                    }

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
                    answer_time,
                    a_leg_sdp,
                    b_leg_sdp,
                    ..
                } => {
                    // Record quality: call answered with PDD
                    if let Some(ref quality) = self.quality {
                        if let Some(state) = self.call_quality_state.get(&call_id.to_string()) {
                            let pdd_ms = answer_time
                                .duration_since(state.initiated_at)
                                .map(|d| d.as_millis() as u64)
                                .unwrap_or(0);
                            quality.record_call_answered(&state.trunk_name, pdd_ms);
                        }
                    }

                    if let Some(ref recorder) = self.recorder {
                        let recorder = Arc::clone(recorder);
                        let call_id_clone = call_id.clone();
                        let a_sdp = a_leg_sdp.clone();
                        let b_sdp = b_leg_sdp.clone();
                        tokio::spawn(async move {
                            recorder
                                .start_recording(&call_id_clone, a_sdp.as_deref(), b_sdp.as_deref())
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
                    duration_ms,
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

                    // Record quality: call ended with duration
                    if let Some(ref quality) = self.quality {
                        if let Some(ref name) = trunk_name {
                            quality.record_call_ended(name, *duration_ms as f64 / 1000.0, 200);
                        }
                    }
                    // Clean up per-call quality state
                    self.call_quality_state.remove(&call_id.to_string());

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
                            let is_trunk_failure =
                                sip_code.map(|c| c >= 500 || c == 408).unwrap_or(true); // no code = likely timeout
                            if is_trunk_failure {
                                health.record_failure(name);
                            }
                        }
                    }

                    // Record quality: call failed
                    if let Some(ref quality) = self.quality {
                        if let Some(ref name) = trunk_name {
                            quality.record_call_failed(name, sip_code.unwrap_or(0));
                        }
                    }
                    // Clean up per-call quality state
                    self.call_quality_state.remove(&call_id.to_string());

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
                RegistrationEvent::Unregistered { .. } | RegistrationEvent::Expired { .. } => {
                    self.admission.registration_removed();
                    debug!("SBC: registration removed, concurrent count decremented");
                }
                RegistrationEvent::Refreshed { .. } | RegistrationEvent::Failed { .. } => {}
            }
        }

        Ok(())
    }
}
