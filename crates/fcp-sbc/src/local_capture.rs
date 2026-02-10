//! Local call recording via forge-media, gated by SIPREC policy.
//!
//! `LocalCaptureManager` bridges the SIPREC recording policy evaluation
//! (All / Selective / None) to the forge-media `MediaManager` for writing
//! RTP streams to local files. Recording metadata is tracked in a
//! `RecordingStore` and exposed via the admin API.

use crate::recording_store::{LocalRecordingStatus, RecordingMetadata, RecordingStore};
use dashmap::DashMap;
use fcp_core::CallId;
use fcp_events::CallDirection;
use fcp_sbc_core::siprec::{RecordingPolicy, SiprecConfig};
use regex::Regex;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{info, warn};
use uuid::Uuid;

/// Pending call info captured at INVITE, consumed when call is answered.
struct PendingCallInfo {
    caller: String,
    callee: String,
}

/// A pre-compiled selective recording rule for local capture matching.
struct CompiledRecordingRule {
    #[allow(dead_code)]
    name: String,
    caller_regex: Option<Regex>,
    callee_regex: Option<Regex>,
    trunk_names: Option<Vec<String>>,
    direction: Option<CallDirection>,
}

/// Manages local call recording: policy evaluation, file capture, and metadata.
pub struct LocalCaptureManager {
    config: SiprecConfig,
    media_manager: Arc<fcp_media::MediaManager>,
    store: Arc<RecordingStore>,
    pending_calls: DashMap<String, PendingCallInfo>,
    compiled_rules: Vec<CompiledRecordingRule>,
}

impl LocalCaptureManager {
    /// Create a new local capture manager.
    ///
    /// Returns `None` if local capture is disabled or the policy is `None`.
    /// Creates the recording directory at startup if it doesn't exist.
    pub fn new(
        config: SiprecConfig,
        media_manager: Arc<fcp_media::MediaManager>,
        store: Arc<RecordingStore>,
    ) -> Option<Self> {
        if !config.local_capture_enabled {
            return None;
        }
        if config.policy == RecordingPolicy::None {
            return None;
        }

        // Create recording directory
        if let Err(e) = std::fs::create_dir_all(&config.recording_dir) {
            warn!(
                dir = %config.recording_dir,
                error = %e,
                "Failed to create recording directory"
            );
        }

        // Pre-compile selective recording rules (same logic as SiprecRecorder)
        let compiled_rules: Vec<CompiledRecordingRule> = config
            .rules
            .iter()
            .filter(|r| r.enabled)
            .filter_map(|r| {
                let caller_regex = r.caller_pattern.as_ref().and_then(|p| {
                    Regex::new(p)
                        .map_err(|e| {
                            warn!(
                                rule = %r.name, pattern = %p, error = %e,
                                "invalid caller_pattern regex in local capture"
                            )
                        })
                        .ok()
                });
                let callee_regex = r.callee_pattern.as_ref().and_then(|p| {
                    Regex::new(p)
                        .map_err(|e| {
                            warn!(
                                rule = %r.name, pattern = %p, error = %e,
                                "invalid callee_pattern regex in local capture"
                            )
                        })
                        .ok()
                });
                let direction = r.direction.as_deref().and_then(|d| match d {
                    "inbound" => Some(CallDirection::Inbound),
                    "outbound" => Some(CallDirection::Outbound),
                    _ => {
                        warn!(
                            rule = %r.name, direction = %d,
                            "invalid direction in local capture rule"
                        );
                        None
                    }
                });
                Some(CompiledRecordingRule {
                    name: r.name.clone(),
                    caller_regex,
                    callee_regex,
                    trunk_names: r.trunk_names.clone(),
                    direction,
                })
            })
            .collect();

        Some(Self {
            config,
            media_manager,
            store,
            pending_calls: DashMap::new(),
            compiled_rules,
        })
    }

    /// Check whether a call should be recorded based on the configured policy.
    pub fn should_record(
        &self,
        from: &str,
        to: &str,
        direction: &CallDirection,
        trunk_name: Option<&str>,
    ) -> bool {
        match self.config.policy {
            RecordingPolicy::None => false,
            RecordingPolicy::All => true,
            RecordingPolicy::Selective => self
                .compiled_rules
                .iter()
                .any(|rule| rule_matches(rule, from, to, direction, trunk_name)),
        }
    }

    /// Called when a new call is initiated. If the call matches recording
    /// policy, stores caller/callee info for later use.
    pub fn on_call_initiated(
        &self,
        call_id: &CallId,
        from: &str,
        to: &str,
        direction: &CallDirection,
        trunk_name: Option<&str>,
    ) {
        if !self.should_record(from, to, direction, trunk_name) {
            return;
        }
        self.pending_calls.insert(
            call_id.to_string(),
            PendingCallInfo {
                caller: from.to_string(),
                callee: to.to_string(),
            },
        );
    }

    /// Called when a call is answered. Starts local file recording and
    /// creates metadata in the store. Errors are logged but never propagate.
    pub async fn on_call_answered(&self, call_id: &CallId) {
        let call_key = call_id.to_string();
        let pending = match self.pending_calls.remove(&call_key) {
            Some((_, info)) => info,
            None => return, // not tracked for recording
        };

        let recording_id = Uuid::new_v4().to_string();
        let file_path = format!(
            "{}/{}.{}",
            self.config.recording_dir, call_key, self.config.recording_format
        );

        let started_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);

        // Insert metadata as InProgress
        self.store.insert(RecordingMetadata {
            recording_id: recording_id.clone(),
            call_id: call_key.clone(),
            caller: pending.caller,
            callee: pending.callee,
            file_path: file_path.clone(),
            format: self.config.recording_format.clone(),
            started_at,
            completed_at: None,
            status: LocalRecordingStatus::InProgress,
        });

        // Start recording via media manager
        match self
            .media_manager
            .start_recording(call_id, file_path.clone())
            .await
        {
            Ok(()) => {
                info!(
                    call_id = %call_key,
                    recording_id = %recording_id,
                    path = %file_path,
                    "Local call recording started"
                );
            }
            Err(e) => {
                warn!(
                    call_id = %call_key,
                    error = %e,
                    path = %file_path,
                    "Failed to start local call recording"
                );
                self.store
                    .mark_failed(&call_key, format!("start failed: {e}"));
            }
        }
    }

    /// Called when a call ends. Stops recording and marks metadata as completed.
    pub async fn on_call_ended(&self, call_id: &CallId) {
        let call_key = call_id.to_string();
        // Remove from pending if never answered
        self.pending_calls.remove(&call_key);

        // Only stop if we have a store entry (i.e., recording was started)
        if self.store.get(&call_key).is_none() {
            return;
        }

        match self.media_manager.stop_recording(call_id).await {
            Ok(()) => {
                self.store.mark_completed(&call_key);
                info!(call_id = %call_key, "Local call recording completed");
            }
            Err(e) => {
                warn!(
                    call_id = %call_key,
                    error = %e,
                    "Failed to stop local call recording"
                );
                self.store
                    .mark_failed(&call_key, format!("stop failed: {e}"));
            }
        }
    }

    /// Called when a call fails before being answered.
    pub fn on_call_failed(&self, call_id: &CallId) {
        let call_key = call_id.to_string();
        self.pending_calls.remove(&call_key);
        // If recording was somehow started (unlikely for failed calls), mark failed
        if self.store.get(&call_key).is_some() {
            self.store
                .mark_failed(&call_key, "call failed".to_string());
        }
    }

    /// Accessor for the recording store (used for wiring into HealthState).
    #[allow(dead_code)]
    pub fn store(&self) -> &Arc<RecordingStore> {
        &self.store
    }
}

/// Check whether a compiled recording rule matches the given call parameters.
fn rule_matches(
    rule: &CompiledRecordingRule,
    from: &str,
    to: &str,
    direction: &CallDirection,
    trunk_name: Option<&str>,
) -> bool {
    if let Some(ref rule_dir) = rule.direction {
        if rule_dir != direction {
            return false;
        }
    }
    if let Some(ref names) = rule.trunk_names {
        match trunk_name {
            Some(tn) => {
                if !names.iter().any(|n| n == tn) {
                    return false;
                }
            }
            None => return false,
        }
    }
    if let Some(ref re) = rule.caller_regex {
        if !re.is_match(from) {
            return false;
        }
    }
    if let Some(ref re) = rule.callee_regex {
        if !re.is_match(to) {
            return false;
        }
    }
    true
}
