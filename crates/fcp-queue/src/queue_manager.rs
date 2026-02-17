//! QueueManager — central runtime state for the ACD/Queue service.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{bail, Result};
use dashmap::DashMap;
use fcp_core::TenantId;
use fcp_events::EventPublisher;
use tracing::{debug, info, warn};

use crate::config::QueueSystemConfig;
use crate::distribution::{self, DistributionPlan};
use crate::events;
use crate::types::{Agent, AgentState, Queue, QueueStats, QueuedCall};

/// Central manager for all queues, agents, and call distribution.
///
/// Thread-safe via `DashMap` — can be shared across tokio tasks.
#[derive(Clone)]
pub struct QueueManager {
    /// Queue name → Queue runtime state.
    queues: Arc<DashMap<String, Queue>>,
    /// Agent ID → Agent runtime state.
    agents: Arc<DashMap<String, Agent>>,
    /// Optional event publisher for queue events.
    events: Option<Arc<dyn EventPublisher>>,
    /// Round-robin counters per queue.
    round_robin_counters: Arc<DashMap<String, AtomicUsize>>,
    /// System configuration.
    config: Arc<parking_lot::RwLock<QueueSystemConfig>>,
    /// Extension → queue name mapping for fast INVITE dispatch.
    extension_map: Arc<DashMap<String, String>>,
    /// Per-queue counters for daily stats.
    calls_handled_today: Arc<DashMap<String, u64>>,
    calls_abandoned_today: Arc<DashMap<String, u64>>,
    calls_within_sl: Arc<DashMap<String, u64>>,
    calls_offered_today: Arc<DashMap<String, u64>>,
}

impl QueueManager {
    /// Create a new QueueManager from configuration.
    pub fn new(
        config: QueueSystemConfig,
        events: Option<Arc<dyn EventPublisher>>,
        tenant_id: TenantId,
    ) -> Self {
        let queues = Arc::new(DashMap::new());
        let extension_map = Arc::new(DashMap::new());
        let round_robin_counters = Arc::new(DashMap::new());
        let calls_handled_today = Arc::new(DashMap::new());
        let calls_abandoned_today = Arc::new(DashMap::new());
        let calls_within_sl = Arc::new(DashMap::new());
        let calls_offered_today = Arc::new(DashMap::new());

        // Initialize queues from config
        for (key, queue_config) in &config.queues {
            let queue = Queue::new(key.clone(), tenant_id.clone(), queue_config.clone());
            extension_map.insert(queue_config.extension.clone(), key.clone());
            round_robin_counters.insert(key.clone(), AtomicUsize::new(0));
            calls_handled_today.insert(key.clone(), 0);
            calls_abandoned_today.insert(key.clone(), 0);
            calls_within_sl.insert(key.clone(), 0);
            calls_offered_today.insert(key.clone(), 0);
            queues.insert(key.clone(), queue);
        }

        Self {
            queues,
            agents: Arc::new(DashMap::new()),
            events,
            round_robin_counters,
            config: Arc::new(parking_lot::RwLock::new(config)),
            extension_map,
            calls_handled_today,
            calls_abandoned_today,
            calls_within_sl,
            calls_offered_today,
        }
    }

    /// Check if a dialed extension maps to a configured queue.
    /// Returns the queue name if found.
    pub fn get_queue_for_extension(&self, extension: &str) -> Option<String> {
        self.extension_map.get(extension).map(|v| v.clone())
    }

    /// Check if the queue system is enabled.
    pub fn is_enabled(&self) -> bool {
        self.config.read().enabled
    }

    // ---- Agent Operations ----

    /// Log an agent into one or more queues.
    pub async fn agent_login(
        &self,
        agent_id: &str,
        extension: &str,
        queues: &[String],
        tenant_id: TenantId,
    ) -> Result<()> {
        // Validate that all requested queues exist
        for q in queues {
            if !self.queues.contains_key(q) {
                bail!("Queue '{}' does not exist", q);
            }
        }

        let mut agent = Agent::new(agent_id.to_string(), extension.to_string(), tenant_id.clone());
        agent.queues = queues.to_vec();
        agent.state = AgentState::Available;
        agent.state_changed_at = Instant::now();

        self.agents.insert(agent_id.to_string(), agent);

        info!(agent_id = %agent_id, queues = ?queues, "Agent logged in");

        for q in queues {
            events::agent_logged_in(&self.events, q, agent_id, &tenant_id).await;
        }

        Ok(())
    }

    /// Log an agent out of all queues.
    pub async fn agent_logout(&self, agent_id: &str) -> Result<()> {
        let mut agent = self
            .agents
            .get_mut(agent_id)
            .ok_or_else(|| anyhow::anyhow!("Agent '{}' not found", agent_id))?;

        let old_state = agent.state.to_string();
        agent.state = AgentState::LoggedOut;
        agent.state_changed_at = Instant::now();
        let tenant_id = agent.tenant_id.clone();
        let agent_queues = agent.queues.clone();
        drop(agent);

        info!(agent_id = %agent_id, "Agent logged out");

        for q in &agent_queues {
            events::agent_logged_out(&self.events, q, agent_id, &tenant_id).await;
        }
        events::agent_state_changed(
            &self.events,
            agent_id,
            &old_state,
            "logged_out",
            &tenant_id,
        )
        .await;

        Ok(())
    }

    /// Set an agent's state (e.g., Available, Away).
    pub async fn set_agent_state(&self, agent_id: &str, state: AgentState) -> Result<()> {
        let mut agent = self
            .agents
            .get_mut(agent_id)
            .ok_or_else(|| anyhow::anyhow!("Agent '{}' not found", agent_id))?;

        let old_state = agent.state.to_string();
        let new_state = state.to_string();
        agent.state = state;
        agent.state_changed_at = Instant::now();
        let tenant_id = agent.tenant_id.clone();
        drop(agent);

        debug!(agent_id = %agent_id, old_state = %old_state, new_state = %new_state, "Agent state changed");

        events::agent_state_changed(&self.events, agent_id, &old_state, &new_state, &tenant_id)
            .await;

        Ok(())
    }

    /// Mark an agent's call as completed. Transitions OnCall → WrapUp and
    /// schedules automatic transition to Available after the wrap-up timeout.
    pub fn agent_call_completed(&self, agent_id: &str) {
        let wrap_up_secs = self.config.read().wrap_up_time_secs;

        if let Some(mut agent) = self.agents.get_mut(agent_id) {
            agent.state = AgentState::WrapUp;
            agent.state_changed_at = Instant::now();
            agent.last_call_ended = Some(Instant::now());
            agent.calls_handled += 1;
            drop(agent);

            // Schedule auto-transition to Available after wrap-up
            let agents = Arc::clone(&self.agents);
            let agent_id = agent_id.to_string();
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(wrap_up_secs as u64)).await;
                if let Some(mut agent) = agents.get_mut(&agent_id) {
                    if agent.state == AgentState::WrapUp {
                        agent.state = AgentState::Available;
                        agent.state_changed_at = Instant::now();
                        debug!(agent_id = %agent_id, "Wrap-up complete, agent now available");
                    }
                }
            });
        } else {
            warn!(agent_id = %agent_id, "agent_call_completed: agent not found");
        }
    }

    // ---- Call Lifecycle ----

    /// Add a call to a queue. Returns the caller's position (1-based).
    pub async fn enqueue_call(&self, call: QueuedCall) -> Result<usize> {
        let queue_name = call.queue_name.clone();
        let call_id = call.call_id.clone();
        let caller_uri = call.caller_uri.clone();
        let tenant_id = call.tenant_id.clone();

        let mut queue = self
            .queues
            .get_mut(&queue_name)
            .ok_or_else(|| anyhow::anyhow!("Queue '{}' does not exist", queue_name))?;

        // Check queue depth
        if queue.waiting_calls.len() >= queue.config.max_queue_depth {
            bail!(
                "Queue '{}' is full ({}/{})",
                queue_name,
                queue.waiting_calls.len(),
                queue.config.max_queue_depth
            );
        }

        queue.insert_call(call);
        let position = queue.waiting_calls.len(); // Position of last inserted
        drop(queue);

        // Update stats
        if let Some(mut count) = self.calls_offered_today.get_mut(&queue_name) {
            *count += 1;
        }

        info!(
            queue = %queue_name,
            call_id = %call_id,
            position = position,
            "Call enqueued"
        );

        events::caller_enqueued(
            &self.events,
            &queue_name,
            &call_id,
            &caller_uri,
            position,
            &tenant_id,
        )
        .await;

        Ok(position)
    }

    /// Remove a call from its queue (e.g., when agent answers).
    /// Returns the dequeued call if found.
    pub fn dequeue_call(&self, call_id: &str) -> Option<QueuedCall> {
        // Search all queues for this call
        for mut queue in self.queues.iter_mut() {
            if let Some(call) = queue.remove_call(call_id) {
                debug!(queue = %queue.name, call_id = %call_id, "Call dequeued");
                return Some(call);
            }
        }
        None
    }

    /// Handle a caller abandoning the queue (hung up while waiting).
    pub async fn caller_abandoned(&self, call_id: &str) {
        for mut queue in self.queues.iter_mut() {
            if let Some(call) = queue.remove_call(call_id) {
                let wait_time = call.enqueued_at.elapsed().as_secs();
                let queue_name = queue.name.clone();
                drop(queue);

                info!(
                    queue = %queue_name,
                    call_id = %call_id,
                    wait_time_secs = wait_time,
                    "Caller abandoned queue"
                );

                if let Some(mut count) = self.calls_abandoned_today.get_mut(&queue_name) {
                    *count += 1;
                }

                events::caller_abandoned(
                    &self.events,
                    &queue_name,
                    call_id,
                    wait_time,
                    call.position,
                    &call.tenant_id,
                )
                .await;

                return;
            }
        }
    }

    // ---- Distribution ----

    /// Select the next agent for a call in the given queue.
    /// Returns None if no agents are available for that queue.
    pub fn select_agent(&self, queue_name: &str) -> Option<DistributionPlan> {
        let queue = self.queues.get(queue_name)?;
        let strategy = queue.config.strategy;
        let ring_timeout = self.config.read().max_ring_time_secs;

        // Collect available agents that belong to this queue
        let available: Vec<&Agent> = Vec::new();
        // We need to collect agent refs, but DashMap refs can't be mixed.
        // Instead, collect agent data we need.
        drop(queue);

        let mut available_agents: Vec<Agent> = Vec::new();
        for entry in self.agents.iter() {
            let agent = entry.value();
            if agent.is_available() && agent.queues.contains(&queue_name.to_string()) {
                available_agents.push(agent.clone());
            }
        }
        drop(available);

        // Sort by priority (lower = higher priority), then by ID for deterministic order
        available_agents.sort_by(|a, b| a.priority.cmp(&b.priority).then(a.id.cmp(&b.id)));

        let refs: Vec<&Agent> = available_agents.iter().collect();

        let rr_index = self
            .round_robin_counters
            .get(queue_name)
            .map(|c| c.load(Ordering::Relaxed))
            .unwrap_or(0);

        let (plan, new_rr) = distribution::select_agent(strategy, &refs, rr_index, ring_timeout)?;

        // Update round-robin counter
        if let Some(counter) = self.round_robin_counters.get(queue_name) {
            counter.store(new_rr, Ordering::Relaxed);
        }

        Some(plan)
    }

    /// Mark an agent as ringing (transitional state — still considered "available"
    /// for tracking but won't be selected again until this attempt resolves).
    pub async fn mark_agent_ringing(&self, agent_id: &str, _call_id: &str) {
        // Transition to OnCall to prevent being selected for another call
        if let Some(mut agent) = self.agents.get_mut(agent_id) {
            agent.state = AgentState::OnCall;
            agent.state_changed_at = Instant::now();
        }
    }

    /// Agent answered a queued call.
    pub async fn agent_answered(&self, agent_id: &str, call_id: &str) {
        if let Some(mut agent) = self.agents.get_mut(agent_id) {
            agent.state = AgentState::OnCall;
            agent.state_changed_at = Instant::now();
            let tenant_id = agent.tenant_id.clone();
            drop(agent);

            // Find and remove the call from its queue
            if let Some(call) = self.dequeue_call(call_id) {
                let wait_time = call.enqueued_at.elapsed().as_secs();
                let queue_name = call.queue_name.clone();

                // Update stats
                if let Some(mut count) = self.calls_handled_today.get_mut(&queue_name) {
                    *count += 1;
                }

                let sl_threshold = self.config.read().service_level_threshold_secs as u64;
                if wait_time <= sl_threshold {
                    if let Some(mut count) = self.calls_within_sl.get_mut(&queue_name) {
                        *count += 1;
                    }
                }

                info!(
                    queue = %queue_name,
                    agent_id = %agent_id,
                    call_id = %call_id,
                    wait_time_secs = wait_time,
                    "Agent answered queued call"
                );

                events::caller_dequeued(
                    &self.events,
                    &queue_name,
                    call_id,
                    agent_id,
                    wait_time,
                    &tenant_id,
                )
                .await;
            }
        }
    }

    /// Agent didn't answer — return call to queue or overflow.
    pub async fn agent_no_answer(&self, agent_id: &str, call_id: &str) {
        // Make agent available again
        if let Some(mut agent) = self.agents.get_mut(agent_id) {
            agent.state = AgentState::Available;
            agent.state_changed_at = Instant::now();
        }

        // The call should still be in the queue (not dequeued yet for no-answer).
        // Increment its attempt counter.
        for mut queue in self.queues.iter_mut() {
            if let Some(call) = queue.waiting_calls.iter_mut().find(|c| c.call_id == call_id) {
                call.attempts += 1;
                let queue_name = queue.name.clone();
                let retry = queue.config.retry_on_no_answer;
                drop(queue);

                if retry {
                    debug!(
                        queue = %queue_name,
                        call_id = %call_id,
                        agent_id = %agent_id,
                        "Agent no-answer, call stays in queue for retry"
                    );
                } else {
                    // Remove and overflow
                    if let Some(mut q) = self.queues.get_mut(&queue_name) {
                        if let Some(removed) = q.remove_call(call_id) {
                            let dest = q
                                .config
                                .overflow_destination
                                .clone()
                                .unwrap_or_default();
                            drop(q);

                            warn!(
                                queue = %queue_name,
                                call_id = %call_id,
                                "Agent no-answer and retry disabled, overflowing"
                            );

                            events::caller_overflowed(
                                &self.events,
                                &queue_name,
                                call_id,
                                &dest,
                                "no_answer_no_retry",
                                &removed.tenant_id,
                            )
                            .await;
                        }
                    }
                }
                return;
            }
        }
    }

    // ---- Statistics ----

    /// Get real-time statistics for a single queue.
    pub fn queue_stats(&self, queue_name: &str) -> Option<QueueStats> {
        let queue = self.queues.get(queue_name)?;
        let now = Instant::now();

        let calls_waiting = queue.waiting_calls.len();
        let (avg_wait, longest_wait) = if calls_waiting > 0 {
            let total_wait: u64 = queue
                .waiting_calls
                .iter()
                .map(|c| now.duration_since(c.enqueued_at).as_secs())
                .sum();
            let longest = queue
                .waiting_calls
                .iter()
                .map(|c| now.duration_since(c.enqueued_at).as_secs())
                .max()
                .unwrap_or(0);
            (total_wait as f64 / calls_waiting as f64, longest)
        } else {
            (0.0, 0)
        };

        // Count agents in each state for this queue
        let mut agents_available = 0usize;
        let mut agents_on_call = 0usize;
        let mut agents_total = 0usize;

        for entry in self.agents.iter() {
            let agent = entry.value();
            if agent.queues.contains(&queue_name.to_string()) && agent.state != AgentState::LoggedOut
            {
                agents_total += 1;
                match agent.state {
                    AgentState::Available => agents_available += 1,
                    AgentState::OnCall => agents_on_call += 1,
                    _ => {}
                }
            }
        }

        let handled = self
            .calls_handled_today
            .get(queue_name)
            .map(|v| *v)
            .unwrap_or(0);
        let abandoned = self
            .calls_abandoned_today
            .get(queue_name)
            .map(|v| *v)
            .unwrap_or(0);
        let within_sl = self
            .calls_within_sl
            .get(queue_name)
            .map(|v| *v)
            .unwrap_or(0);
        let offered = self
            .calls_offered_today
            .get(queue_name)
            .map(|v| *v)
            .unwrap_or(0);

        let service_level_pct = if offered > 0 {
            (within_sl as f64 / offered as f64) * 100.0
        } else {
            100.0
        };

        drop(queue);

        Some(QueueStats {
            queue_name: queue_name.to_string(),
            calls_waiting,
            agents_available,
            agents_on_call,
            agents_total,
            avg_wait_time_secs: avg_wait,
            longest_wait_secs: longest_wait,
            calls_handled_today: handled,
            calls_abandoned_today: abandoned,
            service_level_pct,
        })
    }

    /// Get statistics for all configured queues.
    pub fn all_queue_stats(&self) -> Vec<QueueStats> {
        self.queues
            .iter()
            .filter_map(|entry| self.queue_stats(entry.key()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{DistributionStrategy, QueueConfig, QueueSystemConfig};
    use std::collections::HashMap;

    fn test_tenant() -> TenantId {
        TenantId::new()
    }

    fn test_config() -> QueueSystemConfig {
        let mut queues = HashMap::new();
        queues.insert(
            "sales".to_string(),
            QueueConfig {
                name: "Sales".to_string(),
                extension: "6000".to_string(),
                strategy: DistributionStrategy::RoundRobin,
                max_wait_time_secs: 300,
                max_queue_depth: 5,
                overflow_destination: Some("voicemail:6000".to_string()),
                moh_class: None,
                announce_position: true,
                announce_interval_secs: 30,
                retry_on_no_answer: true,
                agents: vec!["100".to_string(), "101".to_string()],
                priority: 0,
            },
        );
        QueueSystemConfig {
            enabled: true,
            queues,
            wrap_up_time_secs: 5,
            service_level_threshold_secs: 20,
            max_ring_time_secs: 20,
        }
    }

    fn test_queued_call(call_id: &str, queue_name: &str) -> QueuedCall {
        QueuedCall {
            id: uuid::Uuid::new_v4().to_string(),
            call_id: call_id.to_string(),
            queue_name: queue_name.to_string(),
            tenant_id: test_tenant(),
            caller_uri: "sip:caller@example.com".to_string(),
            caller_display: Some("Test Caller".to_string()),
            priority: 0,
            enqueued_at: Instant::now(),
            position: 0,
            attempts: 0,
        }
    }

    #[test]
    fn test_queue_manager_creation() {
        let manager = QueueManager::new(test_config(), None, test_tenant());
        assert!(manager.is_enabled());
        assert_eq!(manager.get_queue_for_extension("6000"), Some("sales".to_string()));
        assert_eq!(manager.get_queue_for_extension("9999"), None);
    }

    #[tokio::test]
    async fn test_agent_login_logout() {
        let manager = QueueManager::new(test_config(), None, test_tenant());
        let tenant = test_tenant();

        // Login
        manager
            .agent_login("100", "100", &["sales".to_string()], tenant.clone())
            .await
            .unwrap();

        // Verify agent is available
        let stats = manager.queue_stats("sales").unwrap();
        assert_eq!(stats.agents_available, 1);
        assert_eq!(stats.agents_total, 1);

        // Logout
        manager.agent_logout("100").await.unwrap();
        let stats = manager.queue_stats("sales").unwrap();
        assert_eq!(stats.agents_available, 0);
        assert_eq!(stats.agents_total, 0);
    }

    #[tokio::test]
    async fn test_agent_login_invalid_queue() {
        let manager = QueueManager::new(test_config(), None, test_tenant());
        let result = manager
            .agent_login("100", "100", &["nonexistent".to_string()], test_tenant())
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_enqueue_dequeue() {
        let manager = QueueManager::new(test_config(), None, test_tenant());

        let call = test_queued_call("call-1", "sales");
        let pos = manager.enqueue_call(call).await.unwrap();
        assert_eq!(pos, 1);

        let stats = manager.queue_stats("sales").unwrap();
        assert_eq!(stats.calls_waiting, 1);

        let dequeued = manager.dequeue_call("call-1");
        assert!(dequeued.is_some());
        assert_eq!(dequeued.unwrap().call_id, "call-1");

        let stats = manager.queue_stats("sales").unwrap();
        assert_eq!(stats.calls_waiting, 0);
    }

    #[tokio::test]
    async fn test_queue_full() {
        let manager = QueueManager::new(test_config(), None, test_tenant());

        // Fill the queue (max_queue_depth = 5)
        for i in 0..5 {
            let call = test_queued_call(&format!("call-{}", i), "sales");
            manager.enqueue_call(call).await.unwrap();
        }

        // 6th call should fail
        let call = test_queued_call("call-overflow", "sales");
        let result = manager.enqueue_call(call).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_select_agent_round_robin() {
        let manager = QueueManager::new(test_config(), None, test_tenant());
        let tenant = test_tenant();

        manager
            .agent_login("100", "100", &["sales".to_string()], tenant.clone())
            .await
            .unwrap();
        manager
            .agent_login("101", "101", &["sales".to_string()], tenant.clone())
            .await
            .unwrap();

        let plan1 = manager.select_agent("sales").unwrap();
        assert_eq!(plan1.agent_id, "100");

        let plan2 = manager.select_agent("sales").unwrap();
        assert_eq!(plan2.agent_id, "101");

        let plan3 = manager.select_agent("sales").unwrap();
        assert_eq!(plan3.agent_id, "100");
    }

    #[tokio::test]
    async fn test_select_agent_none_available() {
        let manager = QueueManager::new(test_config(), None, test_tenant());
        // No agents logged in
        let plan = manager.select_agent("sales");
        assert!(plan.is_none());
    }

    #[tokio::test]
    async fn test_caller_abandoned() {
        let manager = QueueManager::new(test_config(), None, test_tenant());

        let call = test_queued_call("call-1", "sales");
        manager.enqueue_call(call).await.unwrap();

        manager.caller_abandoned("call-1").await;

        let stats = manager.queue_stats("sales").unwrap();
        assert_eq!(stats.calls_waiting, 0);
        assert_eq!(stats.calls_abandoned_today, 1);
    }

    #[tokio::test]
    async fn test_agent_answered_updates_stats() {
        let manager = QueueManager::new(test_config(), None, test_tenant());
        let tenant = test_tenant();

        manager
            .agent_login("100", "100", &["sales".to_string()], tenant.clone())
            .await
            .unwrap();

        let call = test_queued_call("call-1", "sales");
        manager.enqueue_call(call).await.unwrap();

        manager.agent_answered("100", "call-1").await;

        let stats = manager.queue_stats("sales").unwrap();
        assert_eq!(stats.calls_waiting, 0);
        assert_eq!(stats.calls_handled_today, 1);
        assert_eq!(stats.agents_on_call, 1);
    }

    #[test]
    fn test_all_queue_stats() {
        let manager = QueueManager::new(test_config(), None, test_tenant());
        let all_stats = manager.all_queue_stats();
        assert_eq!(all_stats.len(), 1);
        assert_eq!(all_stats[0].queue_name, "sales");
    }

    #[tokio::test]
    async fn test_set_agent_state() {
        let manager = QueueManager::new(test_config(), None, test_tenant());
        let tenant = test_tenant();

        manager
            .agent_login("100", "100", &["sales".to_string()], tenant)
            .await
            .unwrap();

        manager
            .set_agent_state("100", AgentState::Away)
            .await
            .unwrap();

        let stats = manager.queue_stats("sales").unwrap();
        assert_eq!(stats.agents_available, 0);
        assert_eq!(stats.agents_total, 1);

        // Agent marked away should not be selected
        let plan = manager.select_agent("sales");
        assert!(plan.is_none());
    }
}
