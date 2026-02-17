//! Core types for the ACD/Queue service.

use fcp_core::TenantId;
use serde::{Deserialize, Serialize};
use std::time::Instant;

use crate::config::QueueConfig;

/// Agent availability state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    /// Not in the queue — cannot receive calls.
    LoggedOut,
    /// Ready to take calls.
    Available,
    /// Currently handling a call.
    OnCall,
    /// Post-call work period (auto-transitions to Available after wrap-up timeout).
    WrapUp,
    /// Manually marked as away (break, etc.).
    Away,
}

impl std::fmt::Display for AgentState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentState::LoggedOut => write!(f, "logged_out"),
            AgentState::Available => write!(f, "available"),
            AgentState::OnCall => write!(f, "on_call"),
            AgentState::WrapUp => write!(f, "wrap_up"),
            AgentState::Away => write!(f, "away"),
        }
    }
}

/// An agent who can receive queue calls.
#[derive(Debug, Clone)]
pub struct Agent {
    /// Agent ID (usually their extension).
    pub id: String,
    /// SIP extension to ring when distributing a call to this agent.
    pub extension: String,
    /// Current availability state.
    pub state: AgentState,
    /// Tenant this agent belongs to.
    pub tenant_id: TenantId,
    /// Queue names this agent is a member of.
    pub queues: Vec<String>,
    /// Skills for skills-based routing (future use).
    pub skills: Vec<String>,
    /// Agent priority (lower = higher priority).
    pub priority: u32,
    /// Total calls handled (for fewest-calls strategy).
    pub calls_handled: u64,
    /// When the agent's last call ended (for longest-idle strategy).
    pub last_call_ended: Option<Instant>,
    /// When the agent's state last changed.
    pub state_changed_at: Instant,
}

impl Agent {
    /// Create a new agent in the LoggedOut state.
    pub fn new(id: String, extension: String, tenant_id: TenantId) -> Self {
        Self {
            id,
            extension,
            state: AgentState::LoggedOut,
            tenant_id,
            queues: Vec::new(),
            skills: Vec::new(),
            priority: 0,
            calls_handled: 0,
            last_call_ended: None,
            state_changed_at: Instant::now(),
        }
    }

    /// Returns true if the agent can receive a new call.
    pub fn is_available(&self) -> bool {
        self.state == AgentState::Available
    }
}

/// A call waiting in the queue.
#[derive(Debug, Clone)]
pub struct QueuedCall {
    /// Unique queued-call ID.
    pub id: String,
    /// SIP Call-ID.
    pub call_id: String,
    /// Which queue this call is waiting in.
    pub queue_name: String,
    /// Tenant context.
    pub tenant_id: TenantId,
    /// Caller's SIP From URI.
    pub caller_uri: String,
    /// Caller's display name.
    pub caller_display: Option<String>,
    /// Call priority (0 = normal, lower = higher priority).
    pub priority: u32,
    /// When this call was added to the queue.
    pub enqueued_at: Instant,
    /// Current position in the queue (1-based).
    pub position: usize,
    /// How many agent ring attempts have been made.
    pub attempts: u32,
}

/// Runtime state for a single queue instance.
#[derive(Debug)]
pub struct Queue {
    /// Queue name (matches config key).
    pub name: String,
    /// Tenant context.
    pub tenant_id: TenantId,
    /// Queue configuration.
    pub config: QueueConfig,
    /// Calls waiting in this queue, ordered by (priority, enqueue time).
    pub waiting_calls: Vec<QueuedCall>,
}

impl Queue {
    /// Create a new empty queue from configuration.
    pub fn new(name: String, tenant_id: TenantId, config: QueueConfig) -> Self {
        Self {
            name,
            tenant_id,
            config,
            waiting_calls: Vec::new(),
        }
    }

    /// Insert a call in priority order (lower priority value = higher priority).
    /// Among equal priorities, FIFO order is preserved.
    pub fn insert_call(&mut self, mut call: QueuedCall) {
        let pos = self
            .waiting_calls
            .iter()
            .position(|c| c.priority > call.priority)
            .unwrap_or(self.waiting_calls.len());
        call.position = pos + 1;
        self.waiting_calls.insert(pos, call);
        // Recompute positions for all calls after the insertion point
        self.recompute_positions();
    }

    /// Remove a call by its SIP Call-ID.
    pub fn remove_call(&mut self, call_id: &str) -> Option<QueuedCall> {
        if let Some(idx) = self.waiting_calls.iter().position(|c| c.call_id == call_id) {
            let call = self.waiting_calls.remove(idx);
            self.recompute_positions();
            Some(call)
        } else {
            None
        }
    }

    /// Remove and return the highest-priority call (first in the list).
    pub fn pop_next_call(&mut self) -> Option<QueuedCall> {
        if self.waiting_calls.is_empty() {
            None
        } else {
            let call = self.waiting_calls.remove(0);
            self.recompute_positions();
            Some(call)
        }
    }

    /// Re-insert a call at the front of its priority group (e.g., after agent no-answer).
    pub fn reinsert_at_front(&mut self, mut call: QueuedCall) {
        let pos = self
            .waiting_calls
            .iter()
            .position(|c| c.priority >= call.priority)
            .unwrap_or(0);
        call.position = pos + 1;
        self.waiting_calls.insert(pos, call);
        self.recompute_positions();
    }

    fn recompute_positions(&mut self) {
        for (i, call) in self.waiting_calls.iter_mut().enumerate() {
            call.position = i + 1;
        }
    }
}

/// Real-time queue statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueStats {
    /// Queue name.
    pub queue_name: String,
    /// Number of callers currently waiting.
    pub calls_waiting: usize,
    /// Number of agents in Available state.
    pub agents_available: usize,
    /// Number of agents currently on a call.
    pub agents_on_call: usize,
    /// Total agents logged in (any state except LoggedOut).
    pub agents_total: usize,
    /// Average wait time of currently waiting callers (seconds).
    pub avg_wait_time_secs: f64,
    /// Longest current wait time (seconds).
    pub longest_wait_secs: u64,
    /// Total calls handled today.
    pub calls_handled_today: u64,
    /// Total calls abandoned today (caller hung up while waiting).
    pub calls_abandoned_today: u64,
    /// Percentage of calls answered within the service level threshold.
    pub service_level_pct: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{DistributionStrategy, QueueConfig};
    use std::collections::HashMap;

    fn test_tenant() -> TenantId {
        TenantId::new()
    }

    fn test_queue_config() -> QueueConfig {
        QueueConfig {
            name: "Test".to_string(),
            extension: "6000".to_string(),
            strategy: DistributionStrategy::RoundRobin,
            max_wait_time_secs: 300,
            max_queue_depth: 50,
            overflow_destination: None,
            moh_class: None,
            announce_position: true,
            announce_interval_secs: 30,
            retry_on_no_answer: true,
            agents: vec!["100".to_string(), "101".to_string()],
            priority: 0,
            required_skills: Vec::new(),
            agent_skills: HashMap::new(),
        }
    }

    fn test_queued_call(call_id: &str, priority: u32) -> QueuedCall {
        QueuedCall {
            id: uuid::Uuid::new_v4().to_string(),
            call_id: call_id.to_string(),
            queue_name: "test".to_string(),
            tenant_id: test_tenant(),
            caller_uri: "sip:caller@example.com".to_string(),
            caller_display: None,
            priority,
            enqueued_at: Instant::now(),
            position: 0,
            attempts: 0,
        }
    }

    #[test]
    fn test_agent_new() {
        let agent = Agent::new("100".to_string(), "100".to_string(), test_tenant());
        assert_eq!(agent.state, AgentState::LoggedOut);
        assert!(!agent.is_available());
        assert_eq!(agent.calls_handled, 0);
    }

    #[test]
    fn test_queue_insert_fifo() {
        let mut queue = Queue::new("test".to_string(), test_tenant(), test_queue_config());
        queue.insert_call(test_queued_call("call-1", 0));
        queue.insert_call(test_queued_call("call-2", 0));
        queue.insert_call(test_queued_call("call-3", 0));

        assert_eq!(queue.waiting_calls.len(), 3);
        assert_eq!(queue.waiting_calls[0].call_id, "call-1");
        assert_eq!(queue.waiting_calls[1].call_id, "call-2");
        assert_eq!(queue.waiting_calls[2].call_id, "call-3");
        assert_eq!(queue.waiting_calls[0].position, 1);
        assert_eq!(queue.waiting_calls[1].position, 2);
        assert_eq!(queue.waiting_calls[2].position, 3);
    }

    #[test]
    fn test_queue_insert_priority() {
        let mut queue = Queue::new("test".to_string(), test_tenant(), test_queue_config());
        queue.insert_call(test_queued_call("normal-1", 5));
        queue.insert_call(test_queued_call("normal-2", 5));
        queue.insert_call(test_queued_call("high-1", 1));

        assert_eq!(queue.waiting_calls[0].call_id, "high-1");
        assert_eq!(queue.waiting_calls[0].position, 1);
        assert_eq!(queue.waiting_calls[1].call_id, "normal-1");
        assert_eq!(queue.waiting_calls[2].call_id, "normal-2");
    }

    #[test]
    fn test_queue_remove_call() {
        let mut queue = Queue::new("test".to_string(), test_tenant(), test_queue_config());
        queue.insert_call(test_queued_call("call-1", 0));
        queue.insert_call(test_queued_call("call-2", 0));
        queue.insert_call(test_queued_call("call-3", 0));

        let removed = queue.remove_call("call-2");
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().call_id, "call-2");
        assert_eq!(queue.waiting_calls.len(), 2);
        assert_eq!(queue.waiting_calls[0].position, 1);
        assert_eq!(queue.waiting_calls[1].position, 2);
    }

    #[test]
    fn test_queue_pop_next() {
        let mut queue = Queue::new("test".to_string(), test_tenant(), test_queue_config());
        queue.insert_call(test_queued_call("call-1", 0));
        queue.insert_call(test_queued_call("call-2", 0));

        let popped = queue.pop_next_call();
        assert_eq!(popped.unwrap().call_id, "call-1");
        assert_eq!(queue.waiting_calls.len(), 1);
        assert_eq!(queue.waiting_calls[0].position, 1);
    }

    #[test]
    fn test_queue_reinsert_at_front() {
        let mut queue = Queue::new("test".to_string(), test_tenant(), test_queue_config());
        queue.insert_call(test_queued_call("call-1", 0));
        queue.insert_call(test_queued_call("call-2", 0));

        let popped = queue.pop_next_call().unwrap();
        queue.reinsert_at_front(popped);

        assert_eq!(queue.waiting_calls[0].call_id, "call-1");
        assert_eq!(queue.waiting_calls[0].position, 1);
    }

    #[test]
    fn test_agent_state_display() {
        assert_eq!(AgentState::LoggedOut.to_string(), "logged_out");
        assert_eq!(AgentState::Available.to_string(), "available");
        assert_eq!(AgentState::OnCall.to_string(), "on_call");
        assert_eq!(AgentState::WrapUp.to_string(), "wrap_up");
        assert_eq!(AgentState::Away.to_string(), "away");
    }
}
