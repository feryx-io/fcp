//! Call state tracking for aggregating calls across multiple nodes.

use dashmap::DashMap;
use fcp_events::{CallDirection, CallEvent};
use serde::Serialize;
use std::time::{Duration, SystemTime};

/// State of a call in the aggregated view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CallState {
    /// Call initiated (INVITE sent/received)
    Initiated,
    /// Call is ringing (180/183 received)
    Ringing,
    /// Call answered (200 OK)
    Answered,
    /// Call ended normally
    Ended,
    /// Call failed
    Failed,
}

/// Aggregated call information from across all nodes.
#[derive(Debug, Clone, Serialize)]
pub struct AggregatedCall {
    /// Internal call ID
    pub call_id: String,
    /// Node that owns this call
    pub node_id: Option<String>,
    /// Tenant ID
    pub tenant_id: String,
    /// Caller URI/number
    pub from: String,
    /// Callee URI/number
    pub to: String,
    /// Call direction
    pub direction: CallDirection,
    /// Current call state
    pub state: CallState,
    /// When the call was initiated
    #[serde(with = "system_time_serde")]
    pub initiated_at: SystemTime,
    /// When the call started ringing (if applicable)
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "option_system_time_serde"
    )]
    pub ringing_at: Option<SystemTime>,
    /// When the call was answered (if applicable)
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "option_system_time_serde"
    )]
    pub answered_at: Option<SystemTime>,
    /// When the call ended (if applicable)
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "option_system_time_serde"
    )]
    pub ended_at: Option<SystemTime>,
    /// Negotiated codec (if answered)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codec: Option<String>,
    /// Trunk name (if applicable)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trunk_name: Option<String>,
    /// Duration in milliseconds (if ended)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// Hangup cause (if ended)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hangup_cause: Option<String>,
    /// Failure reason (if failed)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
    /// Last update timestamp
    #[serde(with = "system_time_serde")]
    pub last_updated: SystemTime,
}

/// Store for aggregated call state across all nodes.
pub struct CallStateStore {
    calls: DashMap<String, AggregatedCall>,
}

impl CallStateStore {
    /// Create a new call state store.
    pub fn new() -> Self {
        Self {
            calls: DashMap::new(),
        }
    }

    /// Process a call event and update state accordingly.
    pub fn process_event(&self, event: &CallEvent) {
        match event {
            CallEvent::Initiated {
                call_id,
                tenant_id,
                from,
                to,
                direction,
                timestamp,
                trunk_name,
                node_id,
                ..
            } => {
                let call = AggregatedCall {
                    call_id: call_id.to_string(),
                    node_id: node_id.clone(),
                    tenant_id: tenant_id.to_string(),
                    from: from.clone(),
                    to: to.clone(),
                    direction: *direction,
                    state: CallState::Initiated,
                    initiated_at: *timestamp,
                    ringing_at: None,
                    answered_at: None,
                    ended_at: None,
                    codec: None,
                    trunk_name: trunk_name.clone(),
                    duration_ms: None,
                    hangup_cause: None,
                    failure_reason: None,
                    last_updated: SystemTime::now(),
                };
                self.calls.insert(call_id.to_string(), call);
            }
            CallEvent::Ringing {
                call_id,
                timestamp,
                node_id,
            } => {
                if let Some(mut call) = self.calls.get_mut(&call_id.to_string()) {
                    call.state = CallState::Ringing;
                    call.ringing_at = Some(*timestamp);
                    call.last_updated = SystemTime::now();
                    if node_id.is_some() {
                        call.node_id = node_id.clone();
                    }
                }
            }
            CallEvent::Progress {
                call_id,
                timestamp: _,
                node_id,
            } => {
                // Progress events indicate early media; treat similar to ringing
                if let Some(mut call) = self.calls.get_mut(&call_id.to_string()) {
                    if call.state == CallState::Initiated {
                        call.state = CallState::Ringing;
                    }
                    call.last_updated = SystemTime::now();
                    if node_id.is_some() {
                        call.node_id = node_id.clone();
                    }
                }
            }
            CallEvent::Answered {
                call_id,
                answer_time,
                codec,
                node_id,
                ..
            } => {
                if let Some(mut call) = self.calls.get_mut(&call_id.to_string()) {
                    call.state = CallState::Answered;
                    call.answered_at = Some(*answer_time);
                    call.codec = Some(codec.clone());
                    call.last_updated = SystemTime::now();
                    if node_id.is_some() {
                        call.node_id = node_id.clone();
                    }
                }
            }
            CallEvent::Ended {
                call_id,
                end_time,
                duration_ms,
                hangup_cause,
                trunk_name,
                node_id,
                ..
            } => {
                if let Some(mut call) = self.calls.get_mut(&call_id.to_string()) {
                    call.state = CallState::Ended;
                    call.ended_at = Some(*end_time);
                    call.duration_ms = Some(*duration_ms);
                    call.hangup_cause = Some(hangup_cause.clone());
                    if trunk_name.is_some() {
                        call.trunk_name = trunk_name.clone();
                    }
                    call.last_updated = SystemTime::now();
                    if node_id.is_some() {
                        call.node_id = node_id.clone();
                    }
                }
            }
            CallEvent::Failed {
                call_id,
                timestamp,
                reason,
                trunk_name,
                node_id,
                ..
            } => {
                if let Some(mut call) = self.calls.get_mut(&call_id.to_string()) {
                    call.state = CallState::Failed;
                    call.ended_at = Some(*timestamp);
                    call.failure_reason = Some(reason.clone());
                    if trunk_name.is_some() {
                        call.trunk_name = trunk_name.clone();
                    }
                    call.last_updated = SystemTime::now();
                    if node_id.is_some() {
                        call.node_id = node_id.clone();
                    }
                }
            }
            // Parked/Unparked events don't change the aggregated state significantly
            CallEvent::Parked { .. }
            | CallEvent::Unparked { .. }
            | CallEvent::Emergency { .. }
            | CallEvent::EmergencyCompleted { .. }
            | CallEvent::EmergencyNotificationFailed { .. } => {}
        }
    }

    /// Get all active calls (not ended or failed) with calculated duration.
    pub fn get_active_calls(&self) -> Vec<AggregatedCall> {
        self.calls
            .iter()
            .filter(|entry| !matches!(entry.state, CallState::Ended | CallState::Failed))
            .map(|entry| Self::with_calculated_duration(entry.value().clone()))
            .collect()
    }

    /// Get all active calls for a specific node with calculated duration.
    pub fn get_active_calls_by_node(&self, node_id: &str) -> Vec<AggregatedCall> {
        self.calls
            .iter()
            .filter(|entry| {
                !matches!(entry.state, CallState::Ended | CallState::Failed)
                    && entry.node_id.as_deref() == Some(node_id)
            })
            .map(|entry| Self::with_calculated_duration(entry.value().clone()))
            .collect()
    }

    /// Get a specific call by ID with calculated duration.
    pub fn get_call(&self, call_id: &str) -> Option<AggregatedCall> {
        self.calls
            .get(call_id)
            .map(|entry| Self::with_calculated_duration(entry.value().clone()))
    }

    /// Calculate duration for active answered calls.
    /// For ended calls, duration_ms is already set.
    /// For answered calls still in progress, calculate from answered_at.
    fn with_calculated_duration(mut call: AggregatedCall) -> AggregatedCall {
        // Only calculate if not already set (i.e., call is still active)
        if call.duration_ms.is_none() {
            if let Some(answered_at) = call.answered_at {
                if let Ok(duration) = SystemTime::now().duration_since(answered_at) {
                    call.duration_ms = Some(duration.as_millis() as u64);
                }
            }
        }
        call
    }

    /// Get call counts by state.
    pub fn get_stats(&self) -> CallStats {
        let mut stats = CallStats::default();
        for entry in self.calls.iter() {
            stats.total += 1;
            match entry.state {
                CallState::Initiated => stats.initiated += 1,
                CallState::Ringing => stats.ringing += 1,
                CallState::Answered => stats.answered += 1,
                CallState::Ended => stats.ended += 1,
                CallState::Failed => stats.failed += 1,
            }
        }
        stats
    }

    /// Clean up stale calls (ended/failed calls older than max_age, and
    /// active calls stuck in Initiated/Answered state older than max_active_age).
    /// Returns the number of calls removed.
    pub fn cleanup_stale(&self, max_age: Duration) -> usize {
        let now = SystemTime::now();
        let mut removed = 0;
        // Active calls stuck for >1 hour are considered orphaned (missed Ended event)
        let max_active_age = Duration::from_secs(3600);

        self.calls.retain(|_, call| {
            if let Ok(age) = now.duration_since(call.last_updated) {
                // Remove ended/failed calls older than max_age
                if matches!(call.state, CallState::Ended | CallState::Failed) && age > max_age {
                    removed += 1;
                    return false;
                }
                // Remove active calls stuck for too long (orphaned — never received Ended event)
                if !matches!(call.state, CallState::Ended | CallState::Failed)
                    && age > max_active_age
                {
                    tracing::warn!(
                        call_id = %call.call_id,
                        state = ?call.state,
                        age_secs = age.as_secs(),
                        "Removing orphaned active call (no Ended event received)"
                    );
                    removed += 1;
                    return false;
                }
            }
            true
        });

        removed
    }

    /// Get the total number of calls (including ended).
    #[allow(dead_code)]
    pub fn total_count(&self) -> usize {
        self.calls.len()
    }

    /// Get the number of active calls.
    #[allow(dead_code)]
    pub fn active_count(&self) -> usize {
        self.calls
            .iter()
            .filter(|entry| !matches!(entry.state, CallState::Ended | CallState::Failed))
            .count()
    }
}

impl Default for CallStateStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Statistics about calls in the store.
#[derive(Debug, Clone, Default, Serialize)]
pub struct CallStats {
    pub total: usize,
    pub initiated: usize,
    pub ringing: usize,
    pub answered: usize,
    pub ended: usize,
    pub failed: usize,
}

/// Serde module for SystemTime serialization.
mod system_time_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::{SystemTime, UNIX_EPOCH};

    pub fn serialize<S>(time: &SystemTime, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let duration = time
            .duration_since(UNIX_EPOCH)
            .map_err(serde::ser::Error::custom)?;
        duration.as_secs().serialize(serializer)
    }

    #[allow(dead_code)]
    pub fn deserialize<'de, D>(deserializer: D) -> Result<SystemTime, D::Error>
    where
        D: Deserializer<'de>,
    {
        let secs = u64::deserialize(deserializer)?;
        Ok(UNIX_EPOCH + std::time::Duration::from_secs(secs))
    }
}

/// Serde module for Option<SystemTime> serialization.
mod option_system_time_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::{SystemTime, UNIX_EPOCH};

    pub fn serialize<S>(time: &Option<SystemTime>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match time {
            Some(t) => {
                let duration = t
                    .duration_since(UNIX_EPOCH)
                    .map_err(serde::ser::Error::custom)?;
                duration.as_secs().serialize(serializer)
            }
            None => serializer.serialize_none(),
        }
    }

    #[allow(dead_code)]
    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<SystemTime>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let opt: Option<u64> = Option::deserialize(deserializer)?;
        Ok(opt.map(|secs| UNIX_EPOCH + std::time::Duration::from_secs(secs)))
    }
}

// =============================================================================
// Registration State Store
// =============================================================================

use fcp_events::RegistrationEvent;

/// Aggregated registration information from across all nodes.
#[derive(Debug, Clone, Serialize)]
pub struct AggregatedRegistration {
    /// Address of Record (user@domain)
    pub aor: String,
    /// Contact URI
    pub contact: String,
    /// Tenant ID
    pub tenant_id: String,
    /// Node that reported this registration
    pub node_id: Option<String>,
    /// User agent string
    pub user_agent: Option<String>,
    /// Expires in seconds (from last update)
    pub expires_secs: u32,
    /// When the registration was created
    #[serde(with = "system_time_serde")]
    pub registered_at: SystemTime,
    /// Last update timestamp
    #[serde(with = "system_time_serde")]
    pub last_updated: SystemTime,
}

/// Key for registration lookups (aor + contact)
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct RegistrationKey {
    pub aor: String,
    pub contact: String,
}

/// Store for aggregated registration state across all nodes.
pub struct RegistrationStateStore {
    registrations: DashMap<RegistrationKey, AggregatedRegistration>,
}

impl RegistrationStateStore {
    /// Create a new registration state store.
    pub fn new() -> Self {
        Self {
            registrations: DashMap::new(),
        }
    }

    /// Process a registration event and update state accordingly.
    pub fn process_event(&self, event: &RegistrationEvent, node_id: Option<&str>) {
        match event {
            RegistrationEvent::Registered {
                aor,
                contact,
                tenant_id,
                expires_secs,
                user_agent,
                timestamp,
            } => {
                let key = RegistrationKey {
                    aor: aor.clone(),
                    contact: contact.clone(),
                };
                let reg = AggregatedRegistration {
                    aor: aor.clone(),
                    contact: contact.clone(),
                    tenant_id: tenant_id.to_string(),
                    node_id: node_id.map(String::from),
                    user_agent: user_agent.clone(),
                    expires_secs: *expires_secs,
                    registered_at: *timestamp,
                    last_updated: SystemTime::now(),
                };
                self.registrations.insert(key, reg);
            }
            RegistrationEvent::Refreshed {
                aor,
                contact,
                expires_secs,
                timestamp,
                ..
            } => {
                let key = RegistrationKey {
                    aor: aor.clone(),
                    contact: contact.clone(),
                };
                if let Some(mut reg) = self.registrations.get_mut(&key) {
                    reg.expires_secs = *expires_secs;
                    reg.last_updated = *timestamp;
                }
            }
            RegistrationEvent::Unregistered { aor, contact, .. }
            | RegistrationEvent::Expired { aor, contact, .. } => {
                let key = RegistrationKey {
                    aor: aor.clone(),
                    contact: contact.clone(),
                };
                self.registrations.remove(&key);
            }
            RegistrationEvent::Failed { .. } => {
                // Failed registrations don't affect the store
            }
        }
    }

    /// Get all active registrations.
    pub fn get_all_registrations(&self) -> Vec<AggregatedRegistration> {
        self.registrations
            .iter()
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Get registrations for a specific AOR (all contacts).
    #[allow(dead_code)]
    pub fn get_registrations_by_aor(&self, aor: &str) -> Vec<AggregatedRegistration> {
        self.registrations
            .iter()
            .filter(|entry| entry.key().aor == aor)
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Get the total count of active registrations.
    #[allow(dead_code)]
    pub fn count(&self) -> usize {
        self.registrations.len()
    }

    /// Clean up expired registrations.
    /// Registrations are considered expired if last_updated + expires_secs is in the past.
    pub fn cleanup_expired(&self) -> usize {
        let now = SystemTime::now();
        let mut removed = 0;

        self.registrations.retain(|_, reg| {
            let expiry_time = reg.last_updated + Duration::from_secs(reg.expires_secs as u64);
            if expiry_time < now {
                removed += 1;
                false
            } else {
                true
            }
        });

        removed
    }
}

impl Default for RegistrationStateStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fcp_core::{CallId, TenantId};

    fn create_initiated_event(call_id: &str, node_id: Option<&str>) -> CallEvent {
        CallEvent::Initiated {
            call_id: call_id.parse().unwrap(),
            tenant_id: TenantId::nil(),
            from: "sip:alice@example.com".to_string(),
            to: "sip:bob@example.com".to_string(),
            direction: CallDirection::Outbound,
            timestamp: SystemTime::now(),
            trunk_name: None,
            trunk_id: None,
            node_id: node_id.map(String::from),
        }
    }

    #[test]
    fn test_process_initiated() {
        let store = CallStateStore::new();
        let event =
            create_initiated_event("call-12345678-1234-1234-1234-123456789012", Some("node-1"));

        store.process_event(&event);

        assert_eq!(store.total_count(), 1);
        assert_eq!(store.active_count(), 1);

        let call = store
            .get_call("call-12345678-1234-1234-1234-123456789012")
            .unwrap();
        assert_eq!(call.state, CallState::Initiated);
        assert_eq!(call.node_id.as_deref(), Some("node-1"));
    }

    #[test]
    fn test_call_lifecycle() {
        let store = CallStateStore::new();
        let call_id: CallId = "call-12345678-1234-1234-1234-123456789012".parse().unwrap();

        // Initiated
        store.process_event(&create_initiated_event(
            &call_id.to_string(),
            Some("node-1"),
        ));
        assert_eq!(
            store.get_call(&call_id.to_string()).unwrap().state,
            CallState::Initiated
        );

        // Ringing
        store.process_event(&CallEvent::Ringing {
            call_id,
            timestamp: SystemTime::now(),
            node_id: Some("node-1".to_string()),
        });
        assert_eq!(
            store.get_call(&call_id.to_string()).unwrap().state,
            CallState::Ringing
        );

        // Answered
        store.process_event(&CallEvent::Answered {
            call_id,
            answer_time: SystemTime::now(),
            codec: "PCMU".to_string(),
            a_leg_sdp: None,
            b_leg_sdp: None,
            node_id: Some("node-1".to_string()),
        });
        let call = store.get_call(&call_id.to_string()).unwrap();
        assert_eq!(call.state, CallState::Answered);
        assert_eq!(call.codec.as_deref(), Some("PCMU"));

        // Still active
        assert_eq!(store.active_count(), 1);

        // Ended
        store.process_event(&CallEvent::Ended {
            call_id,
            end_time: SystemTime::now(),
            duration_ms: 30000,
            hangup_cause: "Normal Clearing".to_string(),
            hangup_party: None,
            trunk_name: None,
            trunk_id: None,
            node_id: Some("node-1".to_string()),
        });
        let call = store.get_call(&call_id.to_string()).unwrap();
        assert_eq!(call.state, CallState::Ended);
        assert_eq!(call.duration_ms, Some(30000));

        // No longer active
        assert_eq!(store.active_count(), 0);
    }

    #[test]
    fn test_get_active_calls_by_node() {
        let store = CallStateStore::new();

        store.process_event(&create_initiated_event(
            "call-00000000-0000-0000-0000-000000000001",
            Some("node-1"),
        ));
        store.process_event(&create_initiated_event(
            "call-00000000-0000-0000-0000-000000000002",
            Some("node-2"),
        ));
        store.process_event(&create_initiated_event(
            "call-00000000-0000-0000-0000-000000000003",
            Some("node-1"),
        ));

        let node1_calls = store.get_active_calls_by_node("node-1");
        assert_eq!(node1_calls.len(), 2);

        let node2_calls = store.get_active_calls_by_node("node-2");
        assert_eq!(node2_calls.len(), 1);
    }

    #[test]
    fn test_stats() {
        let store = CallStateStore::new();
        let call_id: CallId = "call-12345678-1234-1234-1234-123456789012".parse().unwrap();

        store.process_event(&create_initiated_event(
            &call_id.to_string(),
            Some("node-1"),
        ));

        let stats = store.get_stats();
        assert_eq!(stats.total, 1);
        assert_eq!(stats.initiated, 1);
        assert_eq!(stats.ringing, 0);
        assert_eq!(stats.answered, 0);
    }
}
