//! Call state management for active calls, device states, and BLF.
//!
//! This module provides the core state tracking needed for:
//! - Busy Lamp Field (BLF) / Shared Line Appearance
//! - Hunt group call distribution
//! - Round-robin and longest-idle strategies
//! - Call parking and transfers

use crate::{CallId, DeviceId, LineBindingId, PhoneNumberId, TenantId, UserId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::sync::RwLock;

/// Call state represents the current state of a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CallState {
    /// Call is being set up (dialing)
    Dialing,
    /// Call is ringing at the destination
    Ringing,
    /// Call is in early media (e.g., hearing ringback)
    EarlyMedia,
    /// Call is established and active
    Active,
    /// Call is on hold
    OnHold,
    /// Call is being transferred
    Transferring,
    /// Call is parked
    Parked,
    /// Call is terminated
    Terminated,
}

/// Direction of the call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CallDirection {
    /// Inbound call (from external to internal)
    Inbound,
    /// Outbound call (from internal to external)
    Outbound,
    /// Internal call (between internal endpoints)
    Internal,
}

/// A fork represents one leg of a forked call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallFork {
    /// The device being called
    pub device_id: DeviceId,

    /// SIP dialog ID for this fork
    pub dialog_id: String,

    /// State of this fork
    pub state: CallState,

    /// When this fork started ringing
    pub ring_start: SystemTime,

    /// When this fork was answered (if applicable)
    pub answer_time: Option<SystemTime>,

    /// Priority of this fork
    pub priority: u8,
}

/// Active call information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveCall {
    /// Call ID
    pub call_id: CallId,

    /// Tenant ID
    pub tenant_id: TenantId,

    /// Call state
    pub state: CallState,

    /// Call direction
    pub direction: CallDirection,

    /// Caller information
    pub from_uri: String,
    pub from_display_name: Option<String>,

    /// Called information
    pub to_uri: String,
    pub to_display_name: Option<String>,

    /// The phone number being called (if applicable)
    pub phone_number_id: Option<PhoneNumberId>,

    /// The line binding used for routing
    pub line_binding_id: Option<LineBindingId>,

    /// Owner of the call (if applicable)
    pub owner_user_id: Option<UserId>,

    /// All forked legs of this call
    pub forks: Vec<CallFork>,

    /// The winning fork (the one that answered)
    pub answered_fork: Option<usize>,

    /// When the call was created
    pub created_at: SystemTime,

    /// When the call was answered
    pub answered_at: Option<SystemTime>,

    /// When the call ended
    pub ended_at: Option<SystemTime>,
}

impl ActiveCall {
    /// Creates a new active call.
    pub fn new(
        call_id: CallId,
        tenant_id: TenantId,
        direction: CallDirection,
        from_uri: String,
        to_uri: String,
    ) -> Self {
        Self {
            call_id,
            tenant_id,
            state: CallState::Dialing,
            direction,
            from_uri,
            from_display_name: None,
            to_uri,
            to_display_name: None,
            phone_number_id: None,
            line_binding_id: None,
            owner_user_id: None,
            forks: Vec::new(),
            answered_fork: None,
            created_at: SystemTime::now(),
            answered_at: None,
            ended_at: None,
        }
    }

    /// Returns true if this call is active (not terminated).
    pub fn is_active(&self) -> bool {
        self.state != CallState::Terminated
    }

    /// Returns the number of active forks.
    pub fn active_fork_count(&self) -> usize {
        self.forks.iter().filter(|f| f.state != CallState::Terminated).count()
    }

    /// Returns the answered fork, if any.
    pub fn get_answered_fork(&self) -> Option<&CallFork> {
        self.answered_fork.and_then(|idx| self.forks.get(idx))
    }
}

/// Device state for BLF (Busy Lamp Field).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeviceState {
    /// Device is idle/available
    Idle,
    /// Device is ringing
    Ringing,
    /// Device is on an active call
    Busy,
    /// Device is on hold
    OnHold,
    /// Device is in do-not-disturb mode
    DoNotDisturb,
    /// Device is offline/unregistered
    Offline,
}

/// Device status with timestamp.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceStatus {
    /// Device ID
    pub device_id: DeviceId,

    /// Current state
    pub state: DeviceState,

    /// Number of active calls on this device
    pub active_call_count: usize,

    /// Last state change time
    pub last_state_change: SystemTime,

    /// Last time device was idle (for longest-idle tracking)
    pub last_idle_time: Option<SystemTime>,

    /// Total calls handled (for round-robin tracking)
    pub total_calls_handled: u64,
}

impl DeviceStatus {
    /// Creates a new device status with idle state.
    pub fn new(device_id: DeviceId) -> Self {
        Self {
            device_id,
            state: DeviceState::Idle,
            active_call_count: 0,
            last_state_change: SystemTime::now(),
            last_idle_time: Some(SystemTime::now()),
            total_calls_handled: 0,
        }
    }

    /// Returns true if the device is available to receive calls.
    pub fn is_available(&self) -> bool {
        matches!(self.state, DeviceState::Idle | DeviceState::OnHold)
    }

    /// Returns how long the device has been idle.
    pub fn idle_duration(&self) -> Option<Duration> {
        self.last_idle_time.and_then(|idle_time| {
            SystemTime::now().duration_since(idle_time).ok()
        })
    }
}

/// Call state manager for tracking active calls and device states.
#[derive(Debug, Clone)]
pub struct CallStateManager {
    /// Active calls by call ID
    calls: Arc<RwLock<HashMap<CallId, ActiveCall>>>,

    /// Device states by device ID
    device_states: Arc<RwLock<HashMap<DeviceId, DeviceStatus>>>,

    /// Call index by device ID (for quick lookup)
    device_calls: Arc<RwLock<HashMap<DeviceId, Vec<CallId>>>>,

    /// Round-robin counters by line binding
    round_robin_counters: Arc<RwLock<HashMap<LineBindingId, usize>>>,
}

impl CallStateManager {
    /// Creates a new call state manager.
    pub fn new() -> Self {
        Self {
            calls: Arc::new(RwLock::new(HashMap::new())),
            device_states: Arc::new(RwLock::new(HashMap::new())),
            device_calls: Arc::new(RwLock::new(HashMap::new())),
            round_robin_counters: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Registers a new call.
    pub async fn register_call(&self, call: ActiveCall) {
        let call_id = call.call_id;
        self.calls.write().await.insert(call_id, call);
    }

    /// Gets a call by ID.
    pub async fn get_call(&self, call_id: &CallId) -> Option<ActiveCall> {
        self.calls.read().await.get(call_id).cloned()
    }

    /// Updates a call's state.
    pub async fn update_call_state(&self, call_id: &CallId, state: CallState) {
        if let Some(call) = self.calls.write().await.get_mut(call_id) {
            call.state = state;

            // Update device states based on call state
            for fork in &call.forks {
                self.update_device_state_from_call(&fork.device_id, state).await;
            }
        }
    }

    /// Adds a fork to a call.
    pub async fn add_fork(&self, call_id: &CallId, fork: CallFork) {
        if let Some(call) = self.calls.write().await.get_mut(call_id) {
            let device_id = fork.device_id;
            call.forks.push(fork);

            // Track this device is involved in this call
            self.device_calls.write().await
                .entry(device_id)
                .or_insert_with(Vec::new)
                .push(*call_id);

            // Update device state
            self.update_device_state(&device_id, DeviceState::Ringing).await;
        }
    }

    /// Marks a fork as answered.
    pub async fn answer_fork(&self, call_id: &CallId, fork_index: usize) {
        if let Some(call) = self.calls.write().await.get_mut(call_id) {
            call.answered_fork = Some(fork_index);
            call.answered_at = Some(SystemTime::now());
            call.state = CallState::Active;

            if let Some(fork) = call.forks.get_mut(fork_index) {
                fork.state = CallState::Active;
                fork.answer_time = Some(SystemTime::now());

                // Update device state
                self.update_device_state(&fork.device_id, DeviceState::Busy).await;

                // Increment call counter for round-robin
                if let Some(status) = self.device_states.write().await.get_mut(&fork.device_id) {
                    status.total_calls_handled += 1;
                }
            }

            // Cancel other forks
            for (i, fork) in call.forks.iter_mut().enumerate() {
                if i != fork_index && fork.state != CallState::Terminated {
                    fork.state = CallState::Terminated;
                    self.update_device_state(&fork.device_id, DeviceState::Idle).await;
                }
            }
        }
    }

    /// Terminates a call.
    pub async fn terminate_call(&self, call_id: &CallId) {
        if let Some(call) = self.calls.write().await.get_mut(call_id) {
            call.state = CallState::Terminated;
            call.ended_at = Some(SystemTime::now());

            // Update all device states
            for fork in &mut call.forks {
                if fork.state != CallState::Terminated {
                    fork.state = CallState::Terminated;
                    self.update_device_state(&fork.device_id, DeviceState::Idle).await;
                }
            }

            // Remove from device call tracking
            let mut device_calls = self.device_calls.write().await;
            for fork in &call.forks {
                if let Some(calls) = device_calls.get_mut(&fork.device_id) {
                    calls.retain(|id| id != call_id);
                }
            }
        }
    }

    /// Gets the device state.
    pub async fn get_device_state(&self, device_id: &DeviceId) -> DeviceState {
        self.device_states.read().await
            .get(device_id)
            .map(|s| s.state)
            .unwrap_or(DeviceState::Offline)
    }

    /// Gets full device status.
    pub async fn get_device_status(&self, device_id: &DeviceId) -> Option<DeviceStatus> {
        self.device_states.read().await.get(device_id).cloned()
    }

    /// Updates device state.
    async fn update_device_state(&self, device_id: &DeviceId, state: DeviceState) {
        let mut states = self.device_states.write().await;
        let status = states.entry(*device_id)
            .or_insert_with(|| DeviceStatus::new(*device_id));

        status.state = state;
        status.last_state_change = SystemTime::now();

        // Track idle time
        if state == DeviceState::Idle {
            status.last_idle_time = Some(SystemTime::now());
        }

        // Update active call count
        drop(states);
        let device_calls = self.device_calls.read().await;
        if let Some(calls) = device_calls.get(device_id) {
            let active_count = calls.len();
            let mut states = self.device_states.write().await;
            if let Some(status) = states.get_mut(device_id) {
                status.active_call_count = active_count;
            }
        }
    }

    /// Updates device state based on call state.
    async fn update_device_state_from_call(&self, device_id: &DeviceId, call_state: CallState) {
        let device_state = match call_state {
            CallState::Dialing | CallState::Ringing | CallState::EarlyMedia => DeviceState::Ringing,
            CallState::Active => DeviceState::Busy,
            CallState::OnHold => DeviceState::OnHold,
            CallState::Terminated => DeviceState::Idle,
            CallState::Transferring | CallState::Parked => DeviceState::Busy,
        };

        self.update_device_state(device_id, device_state).await;
    }

    /// Gets all calls for a device.
    pub async fn get_device_calls(&self, device_id: &DeviceId) -> Vec<ActiveCall> {
        let device_calls = self.device_calls.read().await;
        let calls = self.calls.read().await;

        if let Some(call_ids) = device_calls.get(device_id) {
            call_ids.iter()
                .filter_map(|id| calls.get(id).cloned())
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Gets the next device for round-robin distribution.
    pub async fn get_next_round_robin_device(
        &self,
        line_binding_id: &LineBindingId,
        device_ids: &[DeviceId],
    ) -> Option<DeviceId> {
        if device_ids.is_empty() {
            return None;
        }

        let mut counters = self.round_robin_counters.write().await;
        let counter = counters.entry(*line_binding_id).or_insert(0);

        let index = *counter % device_ids.len();
        *counter = (*counter + 1) % device_ids.len();

        device_ids.get(index).copied()
    }

    /// Gets the longest idle device from a list.
    pub async fn get_longest_idle_device(&self, device_ids: &[DeviceId]) -> Option<DeviceId> {
        let states = self.device_states.read().await;

        device_ids.iter()
            .filter_map(|id| states.get(id))
            .filter(|status| status.is_available())
            .max_by_key(|status| status.idle_duration().unwrap_or(Duration::from_secs(0)))
            .map(|status| status.device_id)
    }

    /// Gets all active calls for a tenant.
    pub async fn get_tenant_calls(&self, tenant_id: &TenantId) -> Vec<ActiveCall> {
        self.calls.read().await
            .values()
            .filter(|call| &call.tenant_id == tenant_id && call.is_active())
            .cloned()
            .collect()
    }

    /// Gets call statistics.
    pub async fn get_stats(&self) -> CallStats {
        let calls = self.calls.read().await;
        let device_states = self.device_states.read().await;

        let total_calls = calls.len();
        let active_calls = calls.values().filter(|c| c.is_active()).count();
        let total_devices = device_states.len();
        let busy_devices = device_states.values()
            .filter(|s| s.state == DeviceState::Busy || s.state == DeviceState::Ringing)
            .count();

        CallStats {
            total_calls,
            active_calls,
            total_devices,
            busy_devices,
        }
    }
}

impl Default for CallStateManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Call statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallStats {
    pub total_calls: usize,
    pub active_calls: usize,
    pub total_devices: usize,
    pub busy_devices: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_call_registration() {
        let manager = CallStateManager::new();
        let call_id = CallId::new();
        let tenant_id = TenantId::nil();

        let call = ActiveCall::new(
            call_id,
            tenant_id,
            CallDirection::Inbound,
            "sip:alice@example.com".to_string(),
            "sip:bob@example.com".to_string(),
        );

        manager.register_call(call.clone()).await;

        let retrieved = manager.get_call(&call_id).await;
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().call_id, call_id);
    }

    #[tokio::test]
    async fn test_device_state_tracking() {
        let manager = CallStateManager::new();
        let device_id = DeviceId::new();

        // Initially offline
        assert_eq!(manager.get_device_state(&device_id).await, DeviceState::Offline);

        // Update to ringing
        manager.update_device_state(&device_id, DeviceState::Ringing).await;
        assert_eq!(manager.get_device_state(&device_id).await, DeviceState::Ringing);

        // Update to busy
        manager.update_device_state(&device_id, DeviceState::Busy).await;
        assert_eq!(manager.get_device_state(&device_id).await, DeviceState::Busy);
    }

    #[tokio::test]
    async fn test_round_robin() {
        let manager = CallStateManager::new();
        let line_binding_id = LineBindingId::new();
        let devices = vec![DeviceId::new(), DeviceId::new(), DeviceId::new()];

        // Should cycle through devices
        let d1 = manager.get_next_round_robin_device(&line_binding_id, &devices).await;
        let d2 = manager.get_next_round_robin_device(&line_binding_id, &devices).await;
        let d3 = manager.get_next_round_robin_device(&line_binding_id, &devices).await;
        let d4 = manager.get_next_round_robin_device(&line_binding_id, &devices).await;

        assert_eq!(d1, Some(devices[0]));
        assert_eq!(d2, Some(devices[1]));
        assert_eq!(d3, Some(devices[2]));
        assert_eq!(d4, Some(devices[0])); // Wraps around
    }
}
