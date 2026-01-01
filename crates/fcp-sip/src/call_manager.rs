//! Call manager service for B2BUA.
//!
//! The CallManager tracks all active calls and provides methods for call control.

use crate::call::{Call, CallId, CallState};
use anyhow::{Context, Result};
use dashmap::DashMap;
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
}

impl CallManager {
    /// Create a new call manager.
    pub fn new() -> Self {
        Self {
            calls: Arc::new(DashMap::new()),
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
        let mut entry = self
            .calls
            .get_mut(call_id)
            .context("Call not found")?;

        update_fn(entry.value_mut());

        debug!(
            call_id = %call_id,
            state = ?entry.state,
            "Updated call"
        );

        Ok(entry.value().clone())
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

        // Remove from active calls (in production, might want to keep for CDR)
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
            debug!(dialog_call_id = dialog_call_id, "No call found for dialog Call-ID");
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
    use crate::call::{Call, CallLeg, CallDirection};
    use std::net::SocketAddr;

    fn create_test_call() -> Call {
        use sip_core::{Headers, Method, RequestLine, SipVersion};
        use bytes::Bytes;
        use smol_str::SmolStr;

        let peer: SocketAddr = "127.0.0.1:5060".parse().unwrap();

        let mut headers = Headers::new();
        headers.push(SmolStr::new("From"), SmolStr::new("Alice <sip:alice@example.com>"));
        headers.push(SmolStr::new("To"), SmolStr::new("Bob <sip:bob@example.com>"));
        headers.push(SmolStr::new("Call-ID"), SmolStr::new("test-call-id"));

        let request = sip_core::Request::new(
            RequestLine::new(Method::Invite, sip_core::SipUri::parse("sip:bob@example.com").unwrap()),
            headers,
            Bytes::new(),
        ).expect("Failed to create request");

        Call::from_invite(request, peer)
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
}
