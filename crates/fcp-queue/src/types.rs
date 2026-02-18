//! Core types for the ACD/Queue service.

use fcp_core::TenantId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Instant;

use crate::config::QueueConfig;

/// Trait for checking agent presence/DND status.
///
/// Implement this to let the queue distribution engine skip agents
/// who have set themselves as unavailable via presence (DND, Busy, etc.).
pub trait PresenceChecker: Send + Sync {
    /// Returns true if the agent (identified by extension/AOR) should be
    /// skipped for call distribution due to their presence state.
    fn is_dnd(&self, agent_extension: &str) -> bool;
}

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
    /// Temporarily paused (timed break — auto-resumes after timeout).
    Paused,
}

impl std::fmt::Display for AgentState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentState::LoggedOut => write!(f, "logged_out"),
            AgentState::Available => write!(f, "available"),
            AgentState::OnCall => write!(f, "on_call"),
            AgentState::WrapUp => write!(f, "wrap_up"),
            AgentState::Away => write!(f, "away"),
            AgentState::Paused => write!(f, "paused"),
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
    /// Per-queue priority overrides (queue_name → priority).
    /// Falls back to `priority` (global default) if a queue isn't listed.
    pub queue_priorities: HashMap<String, u32>,
    /// Total calls handled (for fewest-calls strategy).
    pub calls_handled: u64,
    /// When the agent's last call ended (for longest-idle strategy).
    pub last_call_ended: Option<Instant>,
    /// When the agent's state last changed.
    pub state_changed_at: Instant,
    /// Consecutive no-answer count (reset on answer or manual Available).
    pub consecutive_no_answers: u32,
    /// When this agent should auto-resume from Paused state (if paused with a timeout).
    pub pause_resume_at: Option<Instant>,
    /// Cumulative time spent in OnCall state since login (seconds).
    pub total_talk_time_secs: u64,
    /// Cumulative time spent in WrapUp state since login (seconds).
    pub total_wrap_up_time_secs: u64,
    /// Time when the agent logged in (for occupancy calculation).
    pub logged_in_at: Option<Instant>,
    /// Maximum concurrent calls this agent can handle. Default 1.
    pub max_concurrent_calls: u32,
    /// Number of calls currently active for this agent.
    pub current_calls: u32,
    /// Reason for the current Away/Paused state (e.g., "lunch", "meeting", "bathroom").
    pub pause_reason: Option<String>,
    /// Number of wrap-up extensions used for the current call (reset on new call).
    pub wrap_up_extensions_used: u32,
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
            queue_priorities: HashMap::new(),
            calls_handled: 0,
            last_call_ended: None,
            state_changed_at: Instant::now(),
            consecutive_no_answers: 0,
            pause_resume_at: None,
            total_talk_time_secs: 0,
            total_wrap_up_time_secs: 0,
            logged_in_at: None,
            max_concurrent_calls: 1,
            current_calls: 0,
            pause_reason: None,
            wrap_up_extensions_used: 0,
        }
    }

    /// Returns true if the agent can receive a new call.
    pub fn is_available(&self) -> bool {
        match self.state {
            AgentState::Available => true,
            AgentState::OnCall => self.current_calls < self.max_concurrent_calls,
            _ => false,
        }
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
    /// The original priority when enqueued (before escalation).
    pub original_priority: u32,
    /// When this call was added to the queue.
    pub enqueued_at: Instant,
    /// Current position in the queue (1-based).
    pub position: usize,
    /// How many agent ring attempts have been made.
    pub attempts: u32,
    /// Whether this caller has requested a callback (virtual queueing).
    pub is_callback: bool,
    /// URI to call back if `is_callback` is true.
    pub callback_uri: Option<String>,
    /// Number of outbound callback attempts made so far.
    pub callback_attempts: u32,
    /// When the last callback attempt was made (for cooldown enforcement).
    pub last_callback_attempt: Option<Instant>,
}

/// Information about a pending callback for the REST API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallbackInfo {
    pub call_id: String,
    pub queue_name: String,
    pub callback_uri: String,
    pub caller_display: Option<String>,
    pub attempts: u32,
    pub enqueued_at_secs_ago: u64,
    pub last_attempt_secs_ago: Option<u64>,
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
    /// Whether this queue is draining (no new calls accepted, existing calls continue).
    pub draining: bool,
}

impl Queue {
    /// Create a new empty queue from configuration.
    pub fn new(name: String, tenant_id: TenantId, config: QueueConfig) -> Self {
        Self {
            name,
            tenant_id,
            config,
            waiting_calls: Vec::new(),
            draining: false,
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

    /// Escalate call priorities based on wait time.
    /// Calls that have waited longer get their priority decreased (higher urgency).
    pub fn escalate_priorities(&mut self, escalation_secs: u32) {
        if escalation_secs == 0 {
            return;
        }
        let mut changed = false;
        for call in &mut self.waiting_calls {
            let elapsed = call.enqueued_at.elapsed().as_secs() as u32;
            let escalations = elapsed / escalation_secs;
            let new_priority = call.original_priority.saturating_sub(escalations);
            if new_priority != call.priority {
                call.priority = new_priority;
                changed = true;
            }
        }
        if changed {
            self.waiting_calls
                .sort_by(|a, b| a.priority.cmp(&b.priority).then(a.enqueued_at.cmp(&b.enqueued_at)));
            self.recompute_positions();
        }
    }

    pub(crate) fn recompute_positions(&mut self) {
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

/// Per-agent statistics for admin dashboards.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentStats {
    pub agent_id: String,
    pub state: String,
    pub extension: String,
    pub queues: Vec<String>,
    pub calls_handled: u64,
    pub consecutive_no_answers: u32,
    pub total_talk_time_secs: u64,
    pub total_wrap_up_time_secs: u64,
    pub occupancy_pct: f64,
    pub time_in_current_state_secs: u64,
    pub current_calls: u32,
    pub max_concurrent_calls: u32,
    /// Reason for Away/Paused state (e.g., "lunch", "meeting").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pause_reason: Option<String>,
}

/// A number to dial as part of a campaign.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CampaignNumber {
    /// Destination URI or phone number to dial.
    pub destination: String,
    /// Caller ID to present (overrides queue default if set).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caller_id: Option<String>,
    /// Arbitrary metadata to attach to this call (e.g., CRM record ID).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// Outcome of a campaign outbound call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CampaignCallOutcome {
    /// Callee answered and was connected to an agent.
    Answered,
    /// No answer within ring timeout.
    NoAnswer,
    /// Line busy.
    Busy,
    /// Call failed (network error, invalid number, etc.).
    Failed,
    /// Callee answered but no agent was available (over-dial abandon).
    Abandoned,
}

/// Statistics for a campaign run.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CampaignStats {
    /// Numbers remaining in the dial list.
    pub numbers_remaining: usize,
    /// Currently active outbound dials (ringing, not yet answered/failed).
    pub active_dials: usize,
    /// Calls answered and connected to an agent.
    pub calls_answered: u64,
    /// Calls that rang but were not answered.
    pub calls_no_answer: u64,
    /// Calls that got a busy signal.
    pub calls_busy: u64,
    /// Calls that failed (network/invalid).
    pub calls_failed: u64,
    /// Calls answered but abandoned (no agent available).
    pub calls_abandoned: u64,
    /// Whether the campaign is currently running.
    pub running: bool,
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
            overflow_destinations: Vec::new(),
            moh_class: None,
            announce_position: true,
            announce_interval_secs: 30,
            announce_eta: false,
            retry_on_no_answer: true,
            agents: vec!["100".to_string(), "101".to_string()],
            priority: 0,
            required_skills: Vec::new(),
            agent_skills: HashMap::new(),
            dtmf_exit_options: HashMap::new(),
            callback_enabled: false,
            callback_digit: None,
            agent_priorities: HashMap::new(),
            priority_escalation_secs: 0,
            schedule: Vec::new(),
            closed_destination: None,
            timezone: "UTC".to_string(),
            entry_prompt: None,
            connect_prompt: None,
            record_calls: false,
            recording_format: "wav".to_string(),
            callback_max_attempts: 3,
            callback_retry_delay_secs: 60,
            sla_warning_threshold_pct: 0.0,
            agent_weights: HashMap::new(),
            announcement_language: None,
            campaign_enabled: false,
            campaign_dial_ratio: 1.0,
            campaign_abandon_timeout_secs: 30,
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
            original_priority: priority,
            enqueued_at: Instant::now(),
            position: 0,
            attempts: 0,
            is_callback: false,
            callback_uri: None,
            callback_attempts: 0,
            last_callback_attempt: None,
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
        assert_eq!(AgentState::Paused.to_string(), "paused");
    }
}
