//! Call manager service for B2BUA.
//!
//! The CallManager tracks all active calls and provides methods for call control.

use crate::call::{Call, CallDirection, CallId, CallState};
use anyhow::{Context, Result};
use dashmap::DashMap;
use fcp_events::{CallEvent, EventPublisher, PlatformEvent};
use std::sync::Arc;
use tracing::{debug, info};

/// Call manager service.
///
/// This service manages the lifecycle of all active calls in the B2BUA.
/// It provides methods to create, retrieve, update, and terminate calls.
#[derive(Clone)]
pub struct CallManager {
    /// Active calls, indexed by CallId
    calls: Arc<DashMap<CallId, Call>>,

    /// Optional storage backend for CDR creation
    storage: Option<Arc<fcp_core::DynStorage>>,

    /// Optional event publisher for real-time events
    events: Option<Arc<dyn EventPublisher>>,
}

impl CallManager {
    /// Create a new call manager without storage (no CDRs).
    pub fn new() -> Self {
        Self {
            calls: Arc::new(DashMap::new()),
            storage: None,
            events: None,
        }
    }

    /// Create a new call manager with storage backend for CDR creation.
    pub fn with_storage(storage: Arc<fcp_core::DynStorage>) -> Self {
        Self {
            calls: Arc::new(DashMap::new()),
            storage: Some(storage),
            events: None,
        }
    }

    /// Create a new call manager with storage and event publishing.
    pub fn with_storage_and_events(
        storage: Arc<fcp_core::DynStorage>,
        events: Arc<dyn EventPublisher>,
    ) -> Self {
        Self {
            calls: Arc::new(DashMap::new()),
            storage: Some(storage),
            events: Some(events),
        }
    }

    /// Create a new call manager with only event publishing (no CDRs).
    pub fn with_events(events: Arc<dyn EventPublisher>) -> Self {
        Self {
            calls: Arc::new(DashMap::new()),
            storage: None,
            events: Some(events),
        }
    }

    /// Create a new call and store it.
    ///
    /// Returns the CallId for the new call.
    pub fn create_call(&self, call: Call) -> CallId {
        let call_id = call.id;
        info!(
            call_id = %call_id,
            caller = %call.caller(),
            callee = ?call.callee(),
            "Creating new call"
        );

        // Publish CallEvent::Initiated
        if let Some(events) = &self.events {
            let event = PlatformEvent::Call(CallEvent::Initiated {
                call_id: fcp_core::CallId::from_uuid(*call_id.as_uuid()),
                tenant_id: call.tenant_id,
                from: call.a_leg.remote_uri.clone(),
                to: call.callee().unwrap_or("unknown").to_string(),
                direction: match call.a_leg.direction {
                    CallDirection::Inbound => fcp_events::CallDirection::Inbound,
                    CallDirection::Outbound => fcp_events::CallDirection::Outbound,
                },
                timestamp: call.created_at,
            });

            let events = Arc::clone(events);
            tokio::spawn(async move {
                if let Err(e) = events.publish(event).await {
                    tracing::error!(error = %e, "Failed to publish CallEvent::Initiated");
                }
            });
        }

        self.calls.insert(call_id, call);
        call_id
    }

    /// Get a call by ID.
    ///
    /// Returns a clone of the call if it exists.
    pub fn get_call(&self, call_id: &CallId) -> Option<Call> {
        self.calls.get(call_id).map(|entry| entry.value().clone())
    }

    /// Update a call.
    ///
    /// The provided function receives a mutable reference to the call
    /// and can modify it. Returns the updated call.
    pub fn update_call<F>(&self, call_id: &CallId, update_fn: F) -> Result<Call>
    where
        F: FnOnce(&mut Call),
    {
        let mut entry = self.calls.get_mut(call_id).context("Call not found")?;

        // Capture old state for comparison
        let old_state = entry.state;

        update_fn(entry.value_mut());

        let new_state = entry.state;
        let updated_call = entry.value().clone();

        debug!(
            call_id = %call_id,
            old_state = ?old_state,
            new_state = ?new_state,
            "Updated call"
        );

        // Publish state change events
        if let Some(events) = &self.events {
            let should_publish = match (old_state, new_state) {
                // Ringing: transition to Proceeding
                (_, CallState::Proceeding) if old_state != CallState::Proceeding => {
                    Some(PlatformEvent::Call(CallEvent::Ringing {
                        call_id: fcp_core::CallId::from_uuid(*call_id.as_uuid()),
                        timestamp: std::time::SystemTime::now(),
                    }))
                }
                // Answered: transition to Established
                (_, CallState::Established) if old_state != CallState::Established => {
                    // Extract codec from SDP if available
                    let codec = extract_codec_from_sdp(updated_call.a_leg.sdp.as_deref())
                        .unwrap_or_else(|| "unknown".to_string());

                    Some(PlatformEvent::Call(CallEvent::Answered {
                        call_id: fcp_core::CallId::from_uuid(*call_id.as_uuid()),
                        answer_time: updated_call
                            .established_at
                            .unwrap_or_else(std::time::SystemTime::now),
                        codec,
                    }))
                }
                _ => None,
            };

            if let Some(event) = should_publish {
                let events = Arc::clone(events);
                tokio::spawn(async move {
                    if let Err(e) = events.publish(event).await {
                        tracing::error!(error = %e, "Failed to publish state change event");
                    }
                });
            }
        }

        Ok(updated_call)
    }

    /// Terminate a call.
    ///
    /// Marks the call as terminated and removes it from active calls after a delay.
    pub fn terminate_call(&self, call_id: &CallId, reason: Option<String>) -> Result<()> {
        let call = self.update_call(call_id, |call| {
            if let Some(reason) = reason {
                call.fail(reason);
            } else {
                call.terminate();
            }
        })?;

        info!(
            call_id = %call_id,
            caller = %call.caller(),
            callee = ?call.callee(),
            duration = ?call.duration(),
            state = ?call.state,
            "Call terminated"
        );

        // Publish termination event
        if let Some(events) = &self.events {
            let event = match call.state {
                CallState::Terminated => {
                    // Normal termination
                    PlatformEvent::Call(CallEvent::Ended {
                        call_id: fcp_core::CallId::from_uuid(*call_id.as_uuid()),
                        end_time: call.ended_at.unwrap_or_else(std::time::SystemTime::now),
                        duration_ms: call.duration().map(|d| d.as_millis() as u64).unwrap_or(0),
                        hangup_cause: "Normal Clearing".to_string(),
                        hangup_party: None, // TODO: Phase 2 - track BYE direction
                    })
                }
                CallState::Failed => {
                    // Failed call
                    let (reason, sip_code) = call
                        .failure_reason
                        .as_ref()
                        .map(|r| {
                            let code = extract_sip_status_code(r);
                            (r.clone(), code)
                        })
                        .unwrap_or_else(|| ("Unknown error".to_string(), None));

                    PlatformEvent::Call(CallEvent::Failed {
                        call_id: fcp_core::CallId::from_uuid(*call_id.as_uuid()),
                        timestamp: call.ended_at.unwrap_or_else(std::time::SystemTime::now),
                        reason,
                        sip_code,
                    })
                }
                _ => {
                    // Unexpected state - log warning but don't publish event
                    tracing::warn!(
                        call_id = %call_id,
                        state = ?call.state,
                        "Call terminated in unexpected state"
                    );
                    // Publish as Failed with generic reason
                    PlatformEvent::Call(CallEvent::Failed {
                        call_id: fcp_core::CallId::from_uuid(*call_id.as_uuid()),
                        timestamp: std::time::SystemTime::now(),
                        reason: format!("Terminated in unexpected state: {:?}", call.state),
                        sip_code: None,
                    })
                }
            };

            let events = Arc::clone(events);
            tokio::spawn(async move {
                if let Err(e) = events.publish(event).await {
                    tracing::error!(error = %e, "Failed to publish termination event");
                }
            });
        }

        // Create CDR asynchronously if storage is available
        if let Some(storage) = &self.storage {
            let storage = Arc::clone(storage);
            let call_clone = call.clone();
            tokio::spawn(async move {
                if let Err(e) = create_and_insert_cdr(&storage, &call_clone).await {
                    tracing::error!(
                        call_id = %call_clone.id,
                        error = %e,
                        "Failed to create CDR (non-blocking)"
                    );
                } else {
                    tracing::debug!(
                        call_id = %call_clone.id,
                        "CDR created successfully"
                    );
                }
            });
        }

        // Remove from active calls
        self.calls.remove(call_id);

        Ok(())
    }

    /// Get all active calls.
    ///
    /// Returns a vector of all calls that are not in a terminal state.
    pub fn active_calls(&self) -> Vec<Call> {
        self.calls
            .iter()
            .filter(|entry| !entry.value().state.is_terminal())
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Get the count of active calls.
    pub fn active_call_count(&self) -> usize {
        self.calls
            .iter()
            .filter(|entry| !entry.value().state.is_terminal())
            .count()
    }

    /// Get all calls (including terminated).
    pub fn all_calls(&self) -> Vec<Call> {
        self.calls
            .iter()
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Find a call by the SIP Call-ID of the original INVITE.
    pub fn find_by_invite_call_id(&self, sip_call_id: &str) -> Option<Call> {
        self.calls
            .iter()
            .find(|entry| {
                entry
                    .value()
                    .original_invite
                    .as_ref()
                    .and_then(|inv| sip_parse::header(inv.headers(), "Call-ID"))
                    .map(|cid| cid.as_str() == sip_call_id)
                    .unwrap_or(false)
            })
            .map(|entry| entry.value().clone())
    }

    /// Get the total count of calls (including terminated).
    pub fn total_call_count(&self) -> usize {
        self.calls.len()
    }

    /// Find a call by dialog Call-ID.
    ///
    /// This is useful when receiving an in-dialog request (ACK, BYE) where
    /// we need to find the corresponding call from the SIP Call-ID header.
    pub fn find_by_dialog_call_id(&self, dialog_call_id: &str) -> Option<Call> {
        debug!(
            dialog_call_id = dialog_call_id,
            total_calls = self.calls.len(),
            "Looking up call by dialog Call-ID"
        );

        let result = self.calls
            .iter()
            .find(|entry| {
                let call = entry.value();

                debug!(
                    call_id = %call.id,
                    has_a_leg_dialog = call.a_leg.dialog.is_some(),
                    has_b_leg = call.b_leg.is_some(),
                    has_b_leg_dialog = call.b_leg.as_ref().and_then(|b| b.dialog.as_ref()).is_some(),
                    "Checking call"
                );

                // Check A-leg dialog
                if let Some(dialog) = &call.a_leg.dialog {
                    debug!(
                        call_id = %call.id,
                        a_leg_call_id = %dialog.id().call_id(),
                        searching_for = dialog_call_id,
                        "Checking A-leg dialog"
                    );
                    if dialog.id().call_id() == dialog_call_id {
                        debug!(call_id = %call.id, "Found call via A-leg dialog");
                        return true;
                    }
                }

                // Check B-leg dialog
                if let Some(b_leg) = &call.b_leg {
                    if let Some(dialog) = &b_leg.dialog {
                        debug!(
                            call_id = %call.id,
                            b_leg_call_id = %dialog.id().call_id(),
                            searching_for = dialog_call_id,
                            "Checking B-leg dialog"
                        );
                        if dialog.id().call_id() == dialog_call_id {
                            debug!(call_id = %call.id, "Found call via B-leg dialog");
                            return true;
                        }
                    } else {
                        debug!(call_id = %call.id, "B-leg exists but has no dialog");
                    }
                } else {
                    debug!(call_id = %call.id, "Call has no B-leg");
                }

                false
            })
            .map(|entry| entry.value().clone());

        if result.is_none() {
            debug!(
                dialog_call_id = dialog_call_id,
                "No call found for dialog Call-ID"
            );
        }

        result
    }

    /// Clean up terminated calls older than the specified duration.
    ///
    /// This should be called periodically to prevent memory leaks.
    pub fn cleanup_old_calls(&self, max_age: std::time::Duration) -> usize {
        let now = std::time::SystemTime::now();
        let mut removed = 0;

        self.calls.retain(|call_id, call| {
            if call.state.is_terminal() {
                if let Some(ended_at) = call.ended_at {
                    if let Ok(age) = now.duration_since(ended_at) {
                        if age > max_age {
                            debug!(call_id = %call_id, "Cleaning up old terminated call");
                            removed += 1;
                            return false;
                        }
                    }
                }
            }
            true
        });

        if removed > 0 {
            info!(removed = removed, "Cleaned up old terminated calls");
        }

        removed
    }

    /// Get statistics about calls.
    pub fn stats(&self) -> CallStats {
        let all_calls: Vec<_> = self.calls.iter().collect();

        let total = all_calls.len();
        let mut active = 0;
        let mut established = 0;
        let mut terminated = 0;
        let mut failed = 0;

        for entry in all_calls {
            let state = entry.value().state;
            match state {
                CallState::Established => {
                    active += 1;
                    established += 1;
                }
                CallState::Terminated => terminated += 1,
                CallState::Failed => failed += 1,
                _ => active += 1,
            }
        }

        CallStats {
            total,
            active,
            established,
            terminated,
            failed,
        }
    }
}

impl Default for CallManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Create a CDR from a terminated call and insert it into storage.
///
/// This function maps call state to disposition, extracts timing information,
/// and creates a complete CDR record. It is non-blocking and errors are logged
/// but do not prevent call termination.
async fn create_and_insert_cdr(storage: &fcp_core::DynStorage, call: &Call) -> Result<()> {
    use fcp_core::storage::{CallDetailRecord, CdrStore};

    // Map CallState to CallDisposition
    let (disposition, hangup_cause) = map_call_state_to_disposition(call);

    // Extract SIP Call-ID from original INVITE headers
    let sip_call_id = call
        .original_invite
        .as_ref()
        .and_then(|invite| sip_parse::header(invite.headers(), "Call-ID"))
        .map(|cid| cid.as_str().to_string())
        .unwrap_or_else(|| format!("unknown-{}", call.id));

    // Extract SIP status code from failure_reason (e.g., "486 Busy Here" -> 486)
    let sip_status_code = call
        .failure_reason
        .as_ref()
        .and_then(|reason| extract_sip_status_code(reason));

    // Calculate durations in milliseconds
    let (setup_duration_ms, call_duration_ms, total_duration_ms) = calculate_durations(call);

    // Extract caller information (from A-leg)
    let from_uri = call.a_leg.remote_uri.clone();
    let from_display_name = call.a_leg.remote_display_name.clone();

    // Extract callee information (from B-leg or original INVITE To header)
    let (to_uri, to_display_name) = if let Some(b_leg) = &call.b_leg {
        (b_leg.remote_uri.clone(), b_leg.remote_display_name.clone())
    } else {
        // Parse To header from original INVITE
        let to_uri = call
            .original_invite
            .as_ref()
            .and_then(|invite| sip_parse::header(invite.headers(), "To"))
            .map(|to| to.as_str().to_string())
            .unwrap_or_else(|| "unknown".to_string());
        (to_uri, None)
    };

    // Extract codec information from SDP (if available)
    let codec_a_leg = extract_codec_from_sdp(call.a_leg.sdp.as_deref());
    let codec_b_leg = call
        .b_leg
        .as_ref()
        .and_then(|b| extract_codec_from_sdp(b.sdp.as_deref()));

    // Build CallDetailRecord
    let cdr = CallDetailRecord {
        call_id: fcp_core::CallId::from_uuid(*call.id.as_uuid()),
        tenant_id: call.tenant_id,
        sip_call_id,
        from_uri,
        from_display_name,
        to_uri,
        to_display_name,
        start_time: call.created_at,
        answer_time: call.established_at,
        end_time: call.ended_at,
        setup_duration_ms,
        call_duration_ms,
        total_duration_ms,
        disposition,
        hangup_cause,
        hangup_party: None, // TODO: Phase 2 - track BYE direction
        sip_status_code,
        sip_reason: call.failure_reason.clone(),
        route_id: None,
        gateway_id: None,
        media_anchored: call.media_anchored,
        codec_a_leg,
        codec_b_leg,
        rtp_packets_sent: None, // TODO: Phase 2 - RTP statistics
        rtp_packets_received: None,
        rtp_bytes_sent: None,
        rtp_bytes_received: None,
        recording_url: None, // TODO: Phase 2 - recording integration
        cost: None,          // TODO: Phase 4 - billing integration
        currency: None,
        custom_data: None, // No custom data in MVP
    };

    // Insert CDR into storage
    storage
        .insert_cdr(cdr)
        .await
        .context("Failed to insert CDR into storage")?;

    Ok(())
}

/// Map CallState and failure_reason to CallDisposition and HangupCause.
fn map_call_state_to_disposition(
    call: &Call,
) -> (
    fcp_core::storage::CallDisposition,
    fcp_core::storage::HangupCause,
) {
    use fcp_core::storage::{CallDisposition, HangupCause};

    match call.state {
        CallState::Established | CallState::Terminated => {
            // Call was answered
            (CallDisposition::Answered, HangupCause::NormalClearing)
        }
        CallState::Failed => {
            // Check failure_reason for specific error codes
            if let Some(reason) = &call.failure_reason {
                let reason_lower = reason.to_lowercase();

                // Check for 486 Busy
                if reason.contains("486") || reason_lower.contains("busy") {
                    return (CallDisposition::Busy, HangupCause::UserBusy);
                }

                // Check for 487 Cancelled
                if reason.contains("487") || reason_lower.contains("cancel") {
                    return (CallDisposition::Cancelled, HangupCause::NormalClearing);
                }

                // Check for 480/408 No Answer
                if reason.contains("480")
                    || reason.contains("408")
                    || reason_lower.contains("timeout")
                {
                    return (CallDisposition::NoAnswer, HangupCause::NoAnswer);
                }

                // Check for 603 Decline
                if reason.contains("603") || reason_lower.contains("decline") {
                    return (CallDisposition::Failed, HangupCause::CallRejected);
                }

                // Check for 5xx errors
                if reason.starts_with('5') {
                    if let Some(code) = extract_sip_status_code(reason) {
                        if code >= 500 && code < 600 {
                            return (CallDisposition::Failed, HangupCause::InternalError);
                        }
                    }
                }

                // Extract status code for Other cause
                if let Some(code) = extract_sip_status_code(reason) {
                    return (CallDisposition::Failed, HangupCause::Other(code));
                }
            }

            // Generic failure
            (CallDisposition::Failed, HangupCause::Other(0))
        }
        _ => {
            // Should not happen (calls should only be terminated in terminal states)
            tracing::warn!(
                call_id = %call.id,
                state = ?call.state,
                "Unexpected call state during CDR creation"
            );
            (CallDisposition::Failed, HangupCause::Other(0))
        }
    }
}

/// Extract SIP status code from a failure reason string (e.g., "486 Busy Here" -> Some(486)).
fn extract_sip_status_code(reason: &str) -> Option<u16> {
    reason
        .split_whitespace()
        .next()
        .and_then(|code_str| code_str.parse::<u16>().ok())
        .filter(|&code| code >= 100 && code < 700)
}

/// Calculate call durations in milliseconds.
///
/// Returns (setup_duration_ms, call_duration_ms, total_duration_ms):
/// - setup_duration_ms: Time from start to answer (ringing time)
/// - call_duration_ms: Time from answer to end (talk time)
/// - total_duration_ms: Time from start to end
fn calculate_durations(call: &Call) -> (Option<u64>, Option<u64>, Option<u64>) {
    let start = call.created_at;
    let answer = call.established_at;
    let end = call.ended_at;

    // Setup duration: start -> answer
    let setup_duration_ms =
        answer.and_then(|ans| ans.duration_since(start).ok().map(|d| d.as_millis() as u64));

    // Call duration: answer -> end
    let call_duration_ms = answer
        .and_then(|ans| end.and_then(|e| e.duration_since(ans).ok().map(|d| d.as_millis() as u64)));

    // Total duration: start -> end
    let total_duration_ms =
        end.and_then(|e| e.duration_since(start).ok().map(|d| d.as_millis() as u64));

    (setup_duration_ms, call_duration_ms, total_duration_ms)
}

/// Extract codec name from SDP body.
///
/// This is a simple implementation that looks for common codec names in SDP.
/// A more robust implementation would parse the SDP properly.
fn extract_codec_from_sdp(sdp: Option<&str>) -> Option<String> {
    sdp.and_then(|body| {
        // Look for "a=rtpmap:X codec/rate" lines
        for line in body.lines() {
            if line.starts_with("a=rtpmap:") {
                // Parse "a=rtpmap:0 PCMU/8000" -> "PCMU"
                if let Some(codec_part) = line.split_whitespace().nth(1) {
                    if let Some(codec_name) = codec_part.split('/').next() {
                        return Some(codec_name.to_uppercase());
                    }
                }
            }
        }
        None
    })
}

/// Call statistics.
#[derive(Debug, Clone, Copy)]
pub struct CallStats {
    /// Total number of calls (including terminated)
    pub total: usize,

    /// Number of active calls (not in terminal state)
    pub active: usize,

    /// Number of established calls (200 OK received/sent)
    pub established: usize,

    /// Number of terminated calls
    pub terminated: usize,

    /// Number of failed calls
    pub failed: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::call::{Call, CallDirection, CallLeg};
    use std::net::SocketAddr;

    fn create_test_call() -> Call {
        use bytes::Bytes;
        use sip_core::{Headers, Method, RequestLine, SipVersion};
        use smol_str::SmolStr;

        let peer: SocketAddr = "127.0.0.1:5060".parse().unwrap();

        let mut headers = Headers::new();
        headers.push(
            SmolStr::new("From"),
            SmolStr::new("Alice <sip:alice@example.com>"),
        );
        headers.push(
            SmolStr::new("To"),
            SmolStr::new("Bob <sip:bob@example.com>"),
        );
        headers.push(SmolStr::new("Call-ID"), SmolStr::new("test-call-id"));

        let request = sip_core::Request::new(
            RequestLine::new(
                Method::Invite,
                sip_core::SipUri::parse("sip:bob@example.com").unwrap(),
            ),
            headers,
            Bytes::new(),
        )
        .expect("Failed to create request");

        Call::from_invite(request, peer, fcp_core::TenantId::nil())
    }

    #[test]
    fn test_create_and_get_call() {
        let manager = CallManager::new();
        let call = create_test_call();
        let call_id = call.id;

        manager.create_call(call.clone());

        let retrieved = manager.get_call(&call_id);
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().id, call_id);
    }

    #[test]
    fn test_update_call() {
        let manager = CallManager::new();
        let call = create_test_call();
        let call_id = call.id;

        manager.create_call(call);

        let result = manager.update_call(&call_id, |call| {
            call.establish();
        });

        assert!(result.is_ok());
        let updated = manager.get_call(&call_id).unwrap();
        assert_eq!(updated.state, CallState::Established);
    }

    #[test]
    fn test_terminate_call() {
        let manager = CallManager::new();
        let call = create_test_call();
        let call_id = call.id;

        manager.create_call(call);

        let result = manager.terminate_call(&call_id, None);
        assert!(result.is_ok());

        // Call should be removed
        assert!(manager.get_call(&call_id).is_none());
    }

    #[test]
    fn test_active_calls() {
        let manager = CallManager::new();

        // Create active call
        let mut call1 = create_test_call();
        call1.state = CallState::Established;
        manager.create_call(call1);

        // Create terminated call
        let mut call2 = create_test_call();
        call2.terminate();
        manager.create_call(call2);

        assert_eq!(manager.active_call_count(), 1);
        assert_eq!(manager.total_call_count(), 2);
    }

    #[test]
    fn test_stats() {
        let manager = CallManager::new();

        let mut call1 = create_test_call();
        call1.establish();
        manager.create_call(call1);

        let mut call2 = create_test_call();
        call2.terminate();
        manager.create_call(call2);

        let stats = manager.stats();
        assert_eq!(stats.total, 2);
        assert_eq!(stats.established, 1);
        assert_eq!(stats.terminated, 1);
    }

    // ============================================================================
    // CDR State Mapping Tests
    // ============================================================================

    #[test]
    fn test_cdr_state_mapping_answered() {
        use fcp_core::storage::{CallDisposition, HangupCause};

        // Test Established state
        let mut call = create_test_call();
        call.establish();

        let (disposition, cause) = map_call_state_to_disposition(&call);
        assert_eq!(disposition, CallDisposition::Answered);
        assert_eq!(cause, HangupCause::NormalClearing);

        // Test Terminated state (after being established)
        call.terminate();
        let (disposition, cause) = map_call_state_to_disposition(&call);
        assert_eq!(disposition, CallDisposition::Answered);
        assert_eq!(cause, HangupCause::NormalClearing);
    }

    #[test]
    fn test_cdr_state_mapping_busy() {
        use fcp_core::storage::{CallDisposition, HangupCause};

        let mut call = create_test_call();
        call.fail("486 Busy Here".to_string());

        let (disposition, cause) = map_call_state_to_disposition(&call);
        assert_eq!(disposition, CallDisposition::Busy);
        assert_eq!(cause, HangupCause::UserBusy);

        // Test with lowercase "busy" keyword
        let mut call2 = create_test_call();
        call2.fail("User is busy".to_string());

        let (disposition, cause) = map_call_state_to_disposition(&call2);
        assert_eq!(disposition, CallDisposition::Busy);
        assert_eq!(cause, HangupCause::UserBusy);
    }

    #[test]
    fn test_cdr_state_mapping_cancelled() {
        use fcp_core::storage::{CallDisposition, HangupCause};

        let mut call = create_test_call();
        call.fail("487 Request Terminated".to_string());

        let (disposition, cause) = map_call_state_to_disposition(&call);
        assert_eq!(disposition, CallDisposition::Cancelled);
        assert_eq!(cause, HangupCause::NormalClearing);

        // Test with "cancel" keyword
        let mut call2 = create_test_call();
        call2.fail("Call cancelled by user".to_string());

        let (disposition, cause) = map_call_state_to_disposition(&call2);
        assert_eq!(disposition, CallDisposition::Cancelled);
        assert_eq!(cause, HangupCause::NormalClearing);
    }

    #[test]
    fn test_cdr_state_mapping_no_answer() {
        use fcp_core::storage::{CallDisposition, HangupCause};

        // Test 480 Temporarily Unavailable
        let mut call = create_test_call();
        call.fail("480 Temporarily Unavailable".to_string());

        let (disposition, cause) = map_call_state_to_disposition(&call);
        assert_eq!(disposition, CallDisposition::NoAnswer);
        assert_eq!(cause, HangupCause::NoAnswer);

        // Test 408 Request Timeout
        let mut call2 = create_test_call();
        call2.fail("408 Request Timeout".to_string());

        let (disposition, cause) = map_call_state_to_disposition(&call2);
        assert_eq!(disposition, CallDisposition::NoAnswer);
        assert_eq!(cause, HangupCause::NoAnswer);

        // Test with "timeout" keyword
        let mut call3 = create_test_call();
        call3.fail("Transaction timeout".to_string());

        let (disposition, cause) = map_call_state_to_disposition(&call3);
        assert_eq!(disposition, CallDisposition::NoAnswer);
        assert_eq!(cause, HangupCause::NoAnswer);
    }

    #[test]
    fn test_cdr_state_mapping_declined() {
        use fcp_core::storage::{CallDisposition, HangupCause};

        let mut call = create_test_call();
        call.fail("603 Decline".to_string());

        let (disposition, cause) = map_call_state_to_disposition(&call);
        assert_eq!(disposition, CallDisposition::Failed);
        assert_eq!(cause, HangupCause::CallRejected);
    }

    #[test]
    fn test_cdr_state_mapping_server_error() {
        use fcp_core::storage::{CallDisposition, HangupCause};

        let mut call = create_test_call();
        call.fail("500 Server Internal Error".to_string());

        let (disposition, cause) = map_call_state_to_disposition(&call);
        assert_eq!(disposition, CallDisposition::Failed);
        assert_eq!(cause, HangupCause::InternalError);

        // Test 503
        let mut call2 = create_test_call();
        call2.fail("503 Service Unavailable".to_string());

        let (disposition, cause) = map_call_state_to_disposition(&call2);
        assert_eq!(disposition, CallDisposition::Failed);
        assert_eq!(cause, HangupCause::InternalError);
    }

    #[test]
    fn test_cdr_state_mapping_other_failure() {
        use fcp_core::storage::{CallDisposition, HangupCause};

        // Test with SIP status code
        let mut call = create_test_call();
        call.fail("404 Not Found".to_string());

        let (disposition, cause) = map_call_state_to_disposition(&call);
        assert_eq!(disposition, CallDisposition::Failed);
        assert_eq!(cause, HangupCause::Other(404));

        // Test without SIP status code
        let mut call2 = create_test_call();
        call2.fail("Unknown error".to_string());

        let (disposition, cause) = map_call_state_to_disposition(&call2);
        assert_eq!(disposition, CallDisposition::Failed);
        assert_eq!(cause, HangupCause::Other(0));
    }

    // ============================================================================
    // SIP Status Code Extraction Tests
    // ============================================================================

    #[test]
    fn test_extract_sip_status_code_valid() {
        assert_eq!(extract_sip_status_code("486 Busy Here"), Some(486));
        assert_eq!(extract_sip_status_code("200 OK"), Some(200));
        assert_eq!(extract_sip_status_code("404 Not Found"), Some(404));
        assert_eq!(
            extract_sip_status_code("503 Service Unavailable"),
            Some(503)
        );
    }

    #[test]
    fn test_extract_sip_status_code_invalid() {
        // Out of range
        assert_eq!(extract_sip_status_code("99 Invalid"), None);
        assert_eq!(extract_sip_status_code("700 Out of Range"), None);

        // Not a number
        assert_eq!(extract_sip_status_code("ABC Invalid"), None);
        assert_eq!(extract_sip_status_code("Unknown error"), None);

        // Empty
        assert_eq!(extract_sip_status_code(""), None);
    }

    // ============================================================================
    // Duration Calculation Tests
    // ============================================================================

    #[test]
    fn test_calculate_durations_complete_call() {
        use std::time::{Duration, SystemTime};

        let mut call = create_test_call();
        let start = SystemTime::now();
        call.created_at = start;

        // Simulate 2 second setup time
        let answer = start + Duration::from_secs(2);
        call.established_at = Some(answer);

        // Simulate 30 second call duration
        let end = answer + Duration::from_secs(30);
        call.ended_at = Some(end);

        let (setup_ms, call_ms, total_ms) = calculate_durations(&call);

        assert_eq!(setup_ms, Some(2000)); // 2 seconds setup
        assert_eq!(call_ms, Some(30000)); // 30 seconds call
        assert_eq!(total_ms, Some(32000)); // 32 seconds total
    }

    #[test]
    fn test_calculate_durations_unanswered_call() {
        use std::time::{Duration, SystemTime};

        let mut call = create_test_call();
        let start = SystemTime::now();
        call.created_at = start;

        // Call never answered (busy/no answer)
        let end = start + Duration::from_secs(10);
        call.ended_at = Some(end);

        let (setup_ms, call_ms, total_ms) = calculate_durations(&call);

        assert_eq!(setup_ms, None); // Never answered
        assert_eq!(call_ms, None); // No call duration
        assert_eq!(total_ms, Some(10000)); // 10 seconds total (ringing time)
    }

    #[test]
    fn test_calculate_durations_active_call() {
        use std::time::{Duration, SystemTime};

        let mut call = create_test_call();
        let start = SystemTime::now();
        call.created_at = start;

        // Call answered but not ended yet
        let answer = start + Duration::from_secs(3);
        call.established_at = Some(answer);
        call.ended_at = None;

        let (setup_ms, call_ms, total_ms) = calculate_durations(&call);

        assert_eq!(setup_ms, Some(3000)); // 3 seconds setup
        assert_eq!(call_ms, None); // Call still active
        assert_eq!(total_ms, None); // No end time yet
    }

    // ============================================================================
    // Codec Extraction Tests
    // ============================================================================

    #[test]
    fn test_extract_codec_pcmu() {
        let sdp = "v=0\r\n\
                   o=- 123456 789012 IN IP4 192.168.1.1\r\n\
                   s=Test\r\n\
                   c=IN IP4 192.168.1.1\r\n\
                   t=0 0\r\n\
                   m=audio 5004 RTP/AVP 0\r\n\
                   a=rtpmap:0 PCMU/8000\r\n";

        assert_eq!(extract_codec_from_sdp(Some(sdp)), Some("PCMU".to_string()));
    }

    #[test]
    fn test_extract_codec_pcma() {
        let sdp = "v=0\r\n\
                   o=- 123456 789012 IN IP4 192.168.1.1\r\n\
                   s=Test\r\n\
                   c=IN IP4 192.168.1.1\r\n\
                   t=0 0\r\n\
                   m=audio 5004 RTP/AVP 8\r\n\
                   a=rtpmap:8 PCMA/8000\r\n";

        assert_eq!(extract_codec_from_sdp(Some(sdp)), Some("PCMA".to_string()));
    }

    #[test]
    fn test_extract_codec_g722() {
        let sdp = "v=0\r\n\
                   o=- 123456 789012 IN IP4 192.168.1.1\r\n\
                   s=Test\r\n\
                   c=IN IP4 192.168.1.1\r\n\
                   t=0 0\r\n\
                   m=audio 5004 RTP/AVP 9\r\n\
                   a=rtpmap:9 G722/8000\r\n";

        assert_eq!(extract_codec_from_sdp(Some(sdp)), Some("G722".to_string()));
    }

    #[test]
    fn test_extract_codec_opus() {
        let sdp = "v=0\r\n\
                   o=- 123456 789012 IN IP4 192.168.1.1\r\n\
                   s=Test\r\n\
                   c=IN IP4 192.168.1.1\r\n\
                   t=0 0\r\n\
                   m=audio 5004 RTP/AVP 111\r\n\
                   a=rtpmap:111 opus/48000/2\r\n";

        assert_eq!(extract_codec_from_sdp(Some(sdp)), Some("OPUS".to_string()));
    }

    #[test]
    fn test_extract_codec_no_sdp() {
        assert_eq!(extract_codec_from_sdp(None), None);
    }

    #[test]
    fn test_extract_codec_no_rtpmap() {
        let sdp = "v=0\r\n\
                   o=- 123456 789012 IN IP4 192.168.1.1\r\n\
                   s=Test\r\n\
                   c=IN IP4 192.168.1.1\r\n\
                   t=0 0\r\n\
                   m=audio 5004 RTP/AVP 0\r\n";

        assert_eq!(extract_codec_from_sdp(Some(sdp)), None);
    }

    #[test]
    fn test_extract_codec_multiple_rtpmaps() {
        // Should return the first codec found
        let sdp = "v=0\r\n\
                   o=- 123456 789012 IN IP4 192.168.1.1\r\n\
                   s=Test\r\n\
                   c=IN IP4 192.168.1.1\r\n\
                   t=0 0\r\n\
                   m=audio 5004 RTP/AVP 0 8 9\r\n\
                   a=rtpmap:0 PCMU/8000\r\n\
                   a=rtpmap:8 PCMA/8000\r\n\
                   a=rtpmap:9 G722/8000\r\n";

        assert_eq!(extract_codec_from_sdp(Some(sdp)), Some("PCMU".to_string()));
    }
}
