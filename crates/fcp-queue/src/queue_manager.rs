//! QueueManager — central runtime state for the ACD/Queue service.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{bail, Result};
use chrono::Datelike;
use dashmap::DashMap;
use std::collections::VecDeque;

use fcp_core::storage::{QueueStore, StoredAgent, StoredQueueSnapshot};
use fcp_core::TenantId;
use fcp_events::EventPublisher;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use crate::config::{QueueConfig, QueueSystemConfig};
use crate::distribution::{self, DistributionPlan};
use crate::events;
use crate::types::{Agent, AgentState, AgentStats, Queue, QueueStats, QueuedCall};

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
    /// Optional persistent store for agent state.
    queue_store: Option<Arc<dyn QueueStore>>,
    /// In-memory ring buffer of stats snapshots per queue (max 1440 = 24h at 1-min intervals).
    stats_history: Arc<DashMap<String, VecDeque<StoredQueueSnapshot>>>,
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
            queue_store: None,
            stats_history: Arc::new(DashMap::new()),
        }
    }

    /// Set the persistent store for agent state.
    pub fn set_queue_store(&mut self, store: Arc<dyn QueueStore>) {
        self.queue_store = Some(store);
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

    /// Get a reference to the event publisher (for use in external distribution loops).
    pub fn events_ref(&self) -> Option<Arc<dyn EventPublisher>> {
        self.events.clone()
    }

    /// Escalate call priorities for a queue based on wait time.
    pub fn escalate_priorities(&self, queue_name: &str) {
        if let Some(mut queue) = self.queues.get_mut(queue_name) {
            let interval = queue.config.priority_escalation_secs;
            queue.escalate_priorities(interval);
        }
    }

    /// Check if a queue is currently open based on its schedule.
    /// Returns true if no schedule is configured (always open) or if the
    /// current time falls within one of the schedule entries.
    pub fn is_queue_open(&self, queue_name: &str) -> bool {
        let queue = match self.queues.get(queue_name) {
            Some(q) => q,
            None => return false,
        };

        if queue.config.schedule.is_empty() {
            return true; // No schedule = always open
        }

        let tz: chrono_tz::Tz = match queue.config.timezone.parse() {
            Ok(tz) => tz,
            Err(_) => {
                warn!(queue = %queue_name, tz = %queue.config.timezone, "Invalid timezone, treating as always open");
                return true;
            }
        };

        let now = chrono::Utc::now().with_timezone(&tz);
        let current_day = now.weekday().num_days_from_monday() as u8; // 0=Mon..6=Sun
        let current_time = now.format("%H:%M").to_string();

        for entry in &queue.config.schedule {
            if entry.days.contains(&current_day)
                && current_time >= entry.start_time
                && current_time < entry.end_time
            {
                return true;
            }
        }

        false
    }

    /// Transfer a call from one queue to another (queue chaining).
    /// Returns the new position in the target queue.
    pub async fn transfer_to_queue(
        &self,
        call_id: &str,
        from_queue: &str,
        to_queue: &str,
    ) -> Result<usize> {
        // Remove from source queue
        let mut call = {
            let mut source = self
                .queues
                .get_mut(from_queue)
                .ok_or_else(|| anyhow::anyhow!("Source queue '{}' not found", from_queue))?;
            source
                .remove_call(call_id)
                .ok_or_else(|| anyhow::anyhow!("Call '{}' not found in queue '{}'", call_id, from_queue))?
        };

        // Reset for new queue
        call.attempts = 0;
        call.queue_name = to_queue.to_string();

        let tenant_id = call.tenant_id.clone();

        // Insert into target queue
        let mut target = self
            .queues
            .get_mut(to_queue)
            .ok_or_else(|| anyhow::anyhow!("Target queue '{}' not found", to_queue))?;

        if target.waiting_calls.len() >= target.config.max_queue_depth {
            bail!(
                "Target queue '{}' is full ({}/{})",
                to_queue,
                target.waiting_calls.len(),
                target.config.max_queue_depth
            );
        }

        target.insert_call(call);
        let position = target.waiting_calls.len();
        drop(target);

        info!(
            call_id = %call_id,
            from = %from_queue,
            to = %to_queue,
            position = position,
            "Call transferred between queues"
        );

        events::caller_transferred(&self.events, from_queue, to_queue, call_id, &tenant_id).await;

        Ok(position)
    }

    /// Get the closed destination for a queue (closed_destination or overflow_destination).
    pub fn get_closed_destination(&self, queue_name: &str) -> Option<String> {
        self.queues.get(queue_name).and_then(|q| {
            q.config
                .closed_destination
                .clone()
                .or_else(|| q.config.overflow_destination.clone())
        })
    }

    // ---- Drain Operations ----

    /// Start draining a queue — reject new calls, let existing ones finish.
    pub async fn drain_queue(&self, queue_name: &str) -> Result<()> {
        let tenant_id = {
            let mut queue = self
                .queues
                .get_mut(queue_name)
                .ok_or_else(|| anyhow::anyhow!("Queue '{}' not found", queue_name))?;
            queue.draining = true;
            queue.tenant_id.clone()
        };
        events::queue_draining(&self.events, queue_name, &tenant_id).await;
        info!(queue = %queue_name, "Queue draining started");
        Ok(())
    }

    /// Stop draining — resume accepting calls.
    pub async fn undrain_queue(&self, queue_name: &str) -> Result<()> {
        let mut queue = self
            .queues
            .get_mut(queue_name)
            .ok_or_else(|| anyhow::anyhow!("Queue '{}' not found", queue_name))?;
        queue.draining = false;
        drop(queue);
        info!(queue = %queue_name, "Queue draining cancelled");
        Ok(())
    }

    /// Check if a queue is draining.
    pub fn is_queue_draining(&self, queue_name: &str) -> bool {
        self.queues
            .get(queue_name)
            .map(|q| q.draining)
            .unwrap_or(false)
    }

    // ---- Config Reload ----

    /// Reload queue configuration. Adds new queues, updates existing ones.
    /// Does NOT delete queues that are removed from config (to avoid dropping active calls).
    /// Returns list of newly added queue names (for spawning distribution loops).
    pub fn reload_config(&self, new_config: QueueSystemConfig) -> Vec<String> {
        let mut new_queues = Vec::new();

        for (name, qc) in &new_config.queues {
            if self.queues.contains_key(name) {
                // Update existing queue config (preserves waiting_calls, draining state)
                if let Some(mut queue) = self.queues.get_mut(name) {
                    // Update extension mapping if it changed
                    let old_extension = queue.config.extension.clone();
                    if old_extension != qc.extension {
                        self.extension_map.remove(&old_extension);
                        self.extension_map
                            .insert(qc.extension.clone(), name.clone());
                    }
                    queue.config = qc.clone();
                }
            } else {
                // New queue — insert it
                let tenant_id = TenantId::nil();
                self.queues.insert(
                    name.clone(),
                    Queue::new(name.clone(), tenant_id, qc.clone()),
                );
                self.extension_map
                    .insert(qc.extension.clone(), name.clone());
                self.calls_handled_today.insert(name.clone(), 0);
                self.calls_abandoned_today.insert(name.clone(), 0);
                self.calls_within_sl.insert(name.clone(), 0);
                self.calls_offered_today.insert(name.clone(), 0);
                self.round_robin_counters
                    .insert(name.clone(), AtomicUsize::new(0));
                new_queues.push(name.clone());
            }
        }

        // Update system-level config
        *self.config.write() = new_config;

        info!(
            updated = self.queues.len(),
            new = new_queues.len(),
            "Queue configuration reloaded"
        );
        new_queues
    }

    // ---- Agent Operations ----

    /// Log an agent into one or more queues.
    pub async fn agent_login(
        &self,
        agent_id: &str,
        extension: &str,
        queues: &[String],
        tenant_id: TenantId,
        skills: Vec<String>,
    ) -> Result<()> {
        // Validate that all requested queues exist
        for q in queues {
            if !self.queues.contains_key(q) {
                bail!("Queue '{}' does not exist", q);
            }
        }

        let mut agent = Agent::new(agent_id.to_string(), extension.to_string(), tenant_id.clone());
        agent.queues = queues.to_vec();
        agent.skills = skills;
        agent.state = AgentState::Available;
        agent.state_changed_at = Instant::now();
        agent.logged_in_at = Some(Instant::now());
        agent.max_concurrent_calls = self.config.read().default_max_concurrent_calls;

        // Populate per-queue priority overrides from queue configs
        for q_name in queues {
            if let Some(queue) = self.queues.get(q_name) {
                if let Some(&prio) = queue.config.agent_priorities.get(agent_id) {
                    agent.queue_priorities.insert(q_name.clone(), prio);
                }
            }
        }

        self.agents.insert(agent_id.to_string(), agent);

        info!(agent_id = %agent_id, queues = ?queues, "Agent logged in");

        for q in queues {
            events::agent_logged_in(&self.events, q, agent_id, &tenant_id).await;
        }

        self.persist_agent_change(&tenant_id);

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
            None,
            &tenant_id,
        )
        .await;

        self.persist_agent_change(&tenant_id);

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
        // Reset consecutive no-answer counter and clear pause reason when manually set to Available
        if state == AgentState::Available {
            agent.consecutive_no_answers = 0;
            agent.pause_reason = None;
        }
        let tenant_id = agent.tenant_id.clone();
        drop(agent);

        debug!(agent_id = %agent_id, old_state = %old_state, new_state = %new_state, "Agent state changed");

        events::agent_state_changed(&self.events, agent_id, &old_state, &new_state, None, &tenant_id)
            .await;

        self.persist_agent_change(&tenant_id);

        Ok(())
    }

    /// Asynchronously persist all agent state for a tenant.
    /// Spawns a detached task so the caller is not blocked.
    fn persist_agent_change(&self, tenant_id: &TenantId) {
        if let Some(ref store) = self.queue_store {
            let store = Arc::clone(store);
            let tid = tenant_id.clone();
            let agents: Vec<StoredAgent> = self
                .agents
                .iter()
                .filter(|a| a.tenant_id == tid)
                .map(|a| StoredAgent {
                    id: a.id.clone(),
                    extension: a.extension.clone(),
                    queues: a.queues.clone(),
                    skills: a.skills.clone(),
                    priority: a.priority,
                    state: a.state.to_string(),
                    queue_priorities: a.queue_priorities.clone(),
                    max_concurrent_calls: a.max_concurrent_calls,
                })
                .collect();
            tokio::spawn(async move {
                if let Err(e) = store.save_agents(&tid, agents).await {
                    warn!(error = %e, "Failed to persist agent state");
                }
            });
        }
    }

    /// Mark an agent's call as completed. Transitions OnCall → WrapUp and
    /// schedules automatic transition to Available after the wrap-up timeout.
    /// If the agent still has other active calls, stays OnCall.
    pub fn agent_call_completed(&self, agent_id: &str) {
        let wrap_up_secs = self.config.read().wrap_up_time_secs;

        if let Some(mut agent) = self.agents.get_mut(agent_id) {
            agent.current_calls = agent.current_calls.saturating_sub(1);
            agent.calls_handled += 1;
            agent.last_call_ended = Some(Instant::now());

            // Only transition to WrapUp when no more active calls
            if agent.current_calls == 0 {
                // Track talk time before transitioning
                let on_call_duration = agent.state_changed_at.elapsed().as_secs();
                agent.total_talk_time_secs += on_call_duration;

                agent.state = AgentState::WrapUp;
                agent.state_changed_at = Instant::now();
                drop(agent);

                // Schedule auto-transition to Available after wrap-up
                let agents = Arc::clone(&self.agents);
                let agent_id = agent_id.to_string();
                tokio::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(wrap_up_secs as u64)).await;
                    if let Some(mut agent) = agents.get_mut(&agent_id) {
                        if agent.state == AgentState::WrapUp {
                            agent.total_wrap_up_time_secs += agent.state_changed_at.elapsed().as_secs();
                            agent.state = AgentState::Available;
                            agent.state_changed_at = Instant::now();
                            debug!(agent_id = %agent_id, "Wrap-up complete, agent now available");
                        }
                    }
                });
            } else {
                drop(agent);
                debug!(agent_id = %agent_id, "Call completed but agent still has active calls");
            }
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

        // Check if queue is open (schedule check)
        if !self.is_queue_open(&queue_name) {
            bail!("Queue '{}' is currently closed", queue_name);
        }

        // Check if queue is draining
        if self.is_queue_draining(&queue_name) {
            bail!("Queue '{}' is draining — not accepting new calls", queue_name);
        }

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

    /// Check if any higher-priority queue (lower priority number) has waiting calls
    /// that this agent could serve. If so, defer — let that queue's loop claim them.
    pub fn should_defer_agent(&self, agent_id: &str, queue_name: &str) -> bool {
        let this_queue_priority = self
            .queues
            .get(queue_name)
            .map(|q| q.config.priority)
            .unwrap_or(u32::MAX);

        let agent = match self.agents.get(agent_id) {
            Some(a) => a,
            None => return false,
        };

        for other_queue_name in &agent.queues {
            if other_queue_name == queue_name {
                continue;
            }
            if let Some(other_queue) = self.queues.get(other_queue_name) {
                // Skip queues that are lower or equal priority (higher or equal number)
                if other_queue.config.priority >= this_queue_priority {
                    continue;
                }
                // Higher-priority queue has waiting calls — defer this agent
                if !other_queue.waiting_calls.is_empty() {
                    return true;
                }
            }
        }
        false
    }

    /// Select the next agent for a call in the given queue.
    /// Returns None if no agents are available for that queue.
    pub fn select_agent(&self, queue_name: &str) -> Option<DistributionPlan> {
        let queue = self.queues.get(queue_name)?;
        let strategy = queue.config.strategy;
        let required_skills = queue.config.required_skills.clone();
        let agent_weights = queue.config.agent_weights.clone();
        let ring_timeout = self.config.read().max_ring_time_secs;
        drop(queue);

        let mut available_agents: Vec<Agent> = Vec::new();
        for entry in self.agents.iter() {
            let agent = entry.value();
            if agent.is_available() && agent.queues.contains(&queue_name.to_string()) {
                available_agents.push(agent.clone());
            }
        }

        // Filter out agents that should be deferred to higher-priority queues
        available_agents.retain(|agent| !self.should_defer_agent(&agent.id, queue_name));

        // Sort by per-queue priority (lower = higher priority), then by ID for deterministic order
        available_agents.sort_by(|a, b| {
            let a_prio = a.queue_priorities.get(queue_name).copied().unwrap_or(a.priority);
            let b_prio = b.queue_priorities.get(queue_name).copied().unwrap_or(b.priority);
            a_prio.cmp(&b_prio).then(a.id.cmp(&b.id))
        });

        let refs: Vec<&Agent> = available_agents.iter().collect();

        let rr_index = self
            .round_robin_counters
            .get(queue_name)
            .map(|c| c.load(Ordering::Relaxed))
            .unwrap_or(0);

        let (plan, new_rr) =
            distribution::select_agent_weighted(strategy, &refs, rr_index, ring_timeout, &required_skills, &agent_weights)?;

        // Update round-robin counter
        if let Some(counter) = self.round_robin_counters.get(queue_name) {
            counter.store(new_rr, Ordering::Relaxed);
        }

        Some(plan)
    }

    /// Mark an agent as ringing (transitional state — still considered "available"
    /// for tracking but won't be selected again until this attempt resolves).
    pub async fn mark_agent_ringing(&self, agent_id: &str, _call_id: &str) {
        if let Some(mut agent) = self.agents.get_mut(agent_id) {
            agent.current_calls += 1;
            if agent.state != AgentState::OnCall {
                agent.state = AgentState::OnCall;
                agent.state_changed_at = Instant::now();
            }
        }
    }

    /// Agent answered a queued call.
    pub async fn agent_answered(&self, agent_id: &str, call_id: &str) {
        if let Some(mut agent) = self.agents.get_mut(agent_id) {
            agent.state = AgentState::OnCall;
            agent.state_changed_at = Instant::now();
            agent.consecutive_no_answers = 0;
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

    /// Agent's ring was cancelled because the caller hung up.
    /// Marks agent available again (if no other calls) and removes the call from the queue.
    pub async fn agent_no_answer_for_cancel(&self, agent_id: &str, call_id: &str) {
        if let Some(mut agent) = self.agents.get_mut(agent_id) {
            agent.current_calls = agent.current_calls.saturating_sub(1);
            if agent.current_calls == 0 {
                agent.state = AgentState::Available;
                agent.state_changed_at = Instant::now();
            }
        }
        // Remove call from queue if still there
        self.dequeue_call(call_id);
    }

    /// Agent didn't answer — return call to queue or overflow.
    pub async fn agent_no_answer(&self, agent_id: &str, call_id: &str) {
        let max_no_answer = self.config.read().max_no_answer;

        // Update agent state: decrement call count, increment no-answer counter
        if let Some(mut agent) = self.agents.get_mut(agent_id) {
            agent.current_calls = agent.current_calls.saturating_sub(1);
            agent.consecutive_no_answers += 1;
            if max_no_answer > 0 && agent.consecutive_no_answers >= max_no_answer {
                let old_state = agent.state.to_string();
                agent.state = AgentState::Away;
                agent.state_changed_at = Instant::now();
                agent.pause_reason = Some("auto_away_no_answer".to_string());
                let tenant_id = agent.tenant_id.clone();
                drop(agent);
                warn!(
                    agent_id = %agent_id,
                    consecutive_no_answers = max_no_answer,
                    "Agent auto-set to Away after max consecutive no-answers"
                );
                events::agent_state_changed(
                    &self.events,
                    agent_id,
                    &old_state,
                    "away",
                    Some("auto_away_no_answer"),
                    &tenant_id,
                )
                .await;
                self.persist_agent_change(&tenant_id);
            } else if agent.current_calls == 0 {
                agent.state = AgentState::Available;
                agent.state_changed_at = Instant::now();
                drop(agent);
            } else {
                drop(agent);
            }
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

    // ---- Pause / Kick ----

    /// Pause an agent with optional auto-resume duration and reason.
    pub async fn pause_agent(
        &self,
        agent_id: &str,
        duration_secs: Option<u32>,
        reason: Option<String>,
    ) -> Result<()> {
        let mut agent = self
            .agents
            .get_mut(agent_id)
            .ok_or_else(|| anyhow::anyhow!("Agent '{}' not found", agent_id))?;

        let old_state = agent.state.to_string();
        agent.state = AgentState::Paused;
        agent.state_changed_at = Instant::now();
        agent.pause_reason = reason.clone();

        let effective_duration = duration_secs
            .or_else(|| {
                let d = self.config.read().auto_pause_resume_secs;
                if d > 0 { Some(d) } else { None }
            });

        if let Some(secs) = effective_duration {
            agent.pause_resume_at = Some(Instant::now() + Duration::from_secs(secs as u64));
        } else {
            agent.pause_resume_at = None;
        }

        let tenant_id = agent.tenant_id.clone();
        drop(agent);

        info!(agent_id = %agent_id, duration_secs = ?effective_duration, "Agent paused");

        events::agent_state_changed(&self.events, agent_id, &old_state, "paused", reason.as_deref(), &tenant_id)
            .await;
        self.persist_agent_change(&tenant_id);

        // Spawn auto-resume task if duration was provided
        if let Some(secs) = effective_duration {
            let agents = Arc::clone(&self.agents);
            let events = self.events.clone();
            let agent_id_owned = agent_id.to_string();
            let tid = tenant_id.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(secs as u64)).await;
                if let Some(mut agent) = agents.get_mut(&agent_id_owned) {
                    if agent.state == AgentState::Paused {
                        agent.state = AgentState::Available;
                        agent.state_changed_at = Instant::now();
                        agent.pause_resume_at = None;
                        agent.pause_reason = None;
                        drop(agent);
                        debug!(agent_id = %agent_id_owned, "Pause auto-resumed, agent now available");
                        events::agent_state_changed(
                            &events,
                            &agent_id_owned,
                            "paused",
                            "available",
                            None,
                            &tid,
                        )
                        .await;
                    }
                }
            });
        }

        Ok(())
    }

    /// Unpause an agent, returning them to Available.
    pub async fn unpause_agent(&self, agent_id: &str) -> Result<()> {
        let mut agent = self
            .agents
            .get_mut(agent_id)
            .ok_or_else(|| anyhow::anyhow!("Agent '{}' not found", agent_id))?;

        if agent.state != AgentState::Paused {
            bail!("Agent '{}' is not paused (state: {})", agent_id, agent.state);
        }

        agent.state = AgentState::Available;
        agent.state_changed_at = Instant::now();
        agent.pause_resume_at = None;
        agent.pause_reason = None;
        agent.consecutive_no_answers = 0;
        let tenant_id = agent.tenant_id.clone();
        drop(agent);

        info!(agent_id = %agent_id, "Agent unpaused");

        events::agent_state_changed(&self.events, agent_id, "paused", "available", None, &tenant_id)
            .await;
        self.persist_agent_change(&tenant_id);

        Ok(())
    }

    /// Force-logout (kick) an agent from the queue system.
    pub async fn kick_agent(&self, agent_id: &str, reason: &str) -> Result<()> {
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

        info!(agent_id = %agent_id, reason = %reason, "Agent kicked");

        events::agent_kicked(&self.events, agent_id, reason, &tenant_id).await;
        for q in &agent_queues {
            events::agent_logged_out(&self.events, q, agent_id, &tenant_id).await;
        }
        events::agent_state_changed(
            &self.events,
            agent_id,
            &old_state,
            "logged_out",
            Some(reason),
            &tenant_id,
        )
        .await;

        self.persist_agent_change(&tenant_id);

        Ok(())
    }

    // ---- Agent Statistics ----

    /// Get per-agent statistics.
    pub fn agent_stats(&self, agent_id: &str) -> Option<AgentStats> {
        let agent = self.agents.get(agent_id)?;
        let logged_in_duration = agent
            .logged_in_at
            .map(|t| t.elapsed().as_secs())
            .unwrap_or(1);
        let busy_time = agent.total_talk_time_secs + agent.total_wrap_up_time_secs;
        let occupancy = if logged_in_duration > 0 {
            (busy_time as f64 / logged_in_duration as f64) * 100.0
        } else {
            0.0
        };
        Some(AgentStats {
            agent_id: agent.id.clone(),
            state: agent.state.to_string(),
            extension: agent.extension.clone(),
            queues: agent.queues.clone(),
            calls_handled: agent.calls_handled,
            consecutive_no_answers: agent.consecutive_no_answers,
            total_talk_time_secs: agent.total_talk_time_secs,
            total_wrap_up_time_secs: agent.total_wrap_up_time_secs,
            occupancy_pct: occupancy,
            time_in_current_state_secs: agent.state_changed_at.elapsed().as_secs(),
            current_calls: agent.current_calls,
            max_concurrent_calls: agent.max_concurrent_calls,
            pause_reason: agent.pause_reason.clone(),
        })
    }

    /// Get statistics for all agents.
    pub fn all_agent_stats(&self) -> Vec<AgentStats> {
        self.agents
            .iter()
            .filter_map(|entry| self.agent_stats(entry.key()))
            .collect()
    }

    // ---- Query Methods ----

    // ---- Dynamic Queue Provisioning ----

    /// Create a new queue at runtime.
    pub fn create_queue(
        &self,
        key: String,
        queue_config: QueueConfig,
        tenant_id: TenantId,
    ) -> Result<()> {
        if self.queues.contains_key(&key) {
            bail!("Queue '{}' already exists", key);
        }
        let queue = Queue::new(key.clone(), tenant_id, queue_config.clone());
        self.extension_map
            .insert(queue_config.extension.clone(), key.clone());
        self.round_robin_counters
            .insert(key.clone(), AtomicUsize::new(0));
        self.calls_handled_today.insert(key.clone(), 0);
        self.calls_abandoned_today.insert(key.clone(), 0);
        self.calls_within_sl.insert(key.clone(), 0);
        self.calls_offered_today.insert(key.clone(), 0);
        self.queues.insert(key.clone(), queue);

        // Update the system config
        self.config.write().queues.insert(key.clone(), queue_config);

        info!(queue = %key, "Queue created dynamically");
        Ok(())
    }

    /// Update an existing queue's configuration at runtime.
    /// Preserves waiting calls and agent memberships.
    pub fn update_queue(&self, key: &str, queue_config: QueueConfig) -> Result<()> {
        let mut queue = self
            .queues
            .get_mut(key)
            .ok_or_else(|| anyhow::anyhow!("Queue '{}' not found", key))?;

        // Update extension mapping if it changed
        let old_extension = queue.config.extension.clone();
        if old_extension != queue_config.extension {
            self.extension_map.remove(&old_extension);
            self.extension_map
                .insert(queue_config.extension.clone(), key.to_string());
        }

        queue.config = queue_config.clone();
        drop(queue);

        // Update system config
        self.config
            .write()
            .queues
            .insert(key.to_string(), queue_config);

        info!(queue = %key, "Queue configuration updated");
        Ok(())
    }

    /// Delete a queue at runtime. Fails if there are waiting calls.
    pub fn delete_queue(&self, key: &str) -> Result<()> {
        let queue = self
            .queues
            .get(key)
            .ok_or_else(|| anyhow::anyhow!("Queue '{}' not found", key))?;

        if !queue.waiting_calls.is_empty() {
            bail!(
                "Cannot delete queue '{}': {} calls still waiting",
                key,
                queue.waiting_calls.len()
            );
        }

        let extension = queue.config.extension.clone();
        drop(queue);

        self.queues.remove(key);
        self.extension_map.remove(&extension);
        self.round_robin_counters.remove(key);
        self.calls_handled_today.remove(key);
        self.calls_abandoned_today.remove(key);
        self.calls_within_sl.remove(key);
        self.calls_offered_today.remove(key);

        // Remove agents' membership in this queue
        for mut agent in self.agents.iter_mut() {
            agent.queues.retain(|q| q != key);
        }

        // Update system config
        self.config.write().queues.remove(key);

        info!(queue = %key, "Queue deleted");
        Ok(())
    }

    /// List all configured queue names.
    pub fn list_queue_names(&self) -> Vec<String> {
        self.queues.iter().map(|entry| entry.key().clone()).collect()
    }

    /// Get the configuration for a specific queue.
    pub fn get_queue_config(&self, name: &str) -> Option<QueueConfig> {
        self.queues.get(name).map(|q| q.config.clone())
    }

    /// Get the merged overflow destinations for a queue (legacy single + list).
    pub fn get_overflow_destinations(&self, queue_name: &str) -> Vec<String> {
        self.queues
            .get(queue_name)
            .map(|q| q.config.effective_overflow_destinations())
            .unwrap_or_default()
    }

    /// List all agents that are members of a specific queue.
    pub fn list_agents_for_queue(&self, queue_name: &str) -> Vec<Agent> {
        self.agents
            .iter()
            .filter(|entry| entry.value().queues.contains(&queue_name.to_string()))
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Get a specific agent by ID.
    pub fn get_agent(&self, agent_id: &str) -> Option<Agent> {
        self.agents.get(agent_id).map(|a| a.clone())
    }

    /// Get the number of callers waiting in a specific queue.
    pub fn get_queue_waiting_count(&self, queue_name: &str) -> usize {
        self.queues
            .get(queue_name)
            .map(|q| q.waiting_calls.len())
            .unwrap_or(0)
    }

    /// Get the system config (for looking up which queues an agent belongs to).
    pub fn get_system_config(&self) -> QueueSystemConfig {
        self.config.read().clone()
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

    /// Estimate wait time in seconds for a caller based on their queue position,
    /// average handle time of agents, and number of available agents.
    /// Returns None if the caller is not found or no data is available.
    pub fn estimate_wait_secs(&self, call_id: &str) -> Option<u64> {
        // Find the caller's queue and position
        let (queue_name, position) = {
            let mut found = None;
            for queue in self.queues.iter() {
                if let Some(call) = queue.waiting_calls.iter().find(|c| c.call_id == call_id) {
                    found = Some((queue.name.clone(), call.position));
                    break;
                }
            }
            found?
        };

        // Count available agents and compute average handle time for this queue
        let mut agents_available = 0u64;
        let mut total_handle_time = 0u64;
        let mut total_calls = 0u64;

        for entry in self.agents.iter() {
            let agent = entry.value();
            if agent.queues.contains(&queue_name) && agent.state != AgentState::LoggedOut {
                if agent.state == AgentState::Available {
                    agents_available += 1;
                }
                total_handle_time += agent.total_talk_time_secs;
                total_calls += agent.calls_handled;
            }
        }

        if agents_available == 0 || total_calls == 0 {
            return None;
        }

        // Average handle time per call
        let avg_handle = total_handle_time as f64 / total_calls as f64;
        // Add wrap-up time
        let wrap_up = self.config.read().wrap_up_time_secs as f64;
        let avg_cycle = avg_handle + wrap_up;

        // ETA = position * avg_cycle / agents_available
        let eta = (position as f64 * avg_cycle) / agents_available as f64;
        Some(eta.ceil() as u64)
    }

    /// Get the current position of a caller in any queue.
    /// Returns the 1-based position, or None if the call is not found.
    pub fn get_caller_position(&self, call_id: &str) -> Option<usize> {
        for queue in self.queues.iter() {
            if let Some(call) = queue.waiting_calls.iter().find(|c| c.call_id == call_id) {
                return Some(call.position);
            }
        }
        None
    }

    /// Check SLA thresholds for all queues and emit warnings for any below threshold.
    pub async fn check_sla_warnings(&self, tenant_id: &TenantId) {
        for queue in self.queues.iter() {
            let threshold = queue.config.sla_warning_threshold_pct;
            if threshold <= 0.0 {
                continue;
            }
            let queue_name = queue.name.clone();
            drop(queue);

            let offered = self
                .calls_offered_today
                .get(&queue_name)
                .map(|v| *v)
                .unwrap_or(0);
            let within_sl = self
                .calls_within_sl
                .get(&queue_name)
                .map(|v| *v)
                .unwrap_or(0);

            if offered == 0 {
                continue; // No calls yet, skip
            }

            let current_pct = (within_sl as f64 / offered as f64) * 100.0;
            if current_pct < threshold {
                warn!(
                    queue = %queue_name,
                    current_pct = %format!("{:.1}", current_pct),
                    threshold_pct = %format!("{:.1}", threshold),
                    "Service level below warning threshold"
                );
                events::service_level_warning(
                    &self.events,
                    &queue_name,
                    current_pct,
                    threshold,
                    offered,
                    within_sl,
                    tenant_id,
                )
                .await;
            }
        }
    }

    /// Record stats snapshots to the in-memory ring buffer (and persistent store if available).
    async fn record_stats_snapshots(&self, stats: &[QueueStats], tenant_id: &TenantId) {
        let now_epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        for s in stats {
            let snapshot = StoredQueueSnapshot {
                queue_name: s.queue_name.clone(),
                timestamp_epoch: now_epoch,
                calls_waiting: s.calls_waiting,
                agents_available: s.agents_available,
                agents_on_call: s.agents_on_call,
                agents_total: s.agents_total,
                avg_wait_time_secs: s.avg_wait_time_secs,
                longest_wait_secs: s.longest_wait_secs,
                calls_handled_today: s.calls_handled_today,
                calls_abandoned_today: s.calls_abandoned_today,
                service_level_pct: s.service_level_pct,
            };

            // Save to in-memory ring buffer (max 1440 entries = 24h at 1-min intervals)
            let mut buffer = self
                .stats_history
                .entry(s.queue_name.clone())
                .or_insert_with(VecDeque::new);
            if buffer.len() >= 1440 {
                buffer.pop_front();
            }
            buffer.push_back(snapshot.clone());
            drop(buffer);

            // Persist to storage if available
            if let Some(ref store) = self.queue_store {
                let store = Arc::clone(store);
                let tid = tenant_id.clone();
                tokio::spawn(async move {
                    if let Err(e) = store.save_stats_snapshot(&tid, snapshot).await {
                        warn!(error = %e, "Failed to persist stats snapshot");
                    }
                });
            }
        }
    }

    /// Query historical stats snapshots from in-memory buffer.
    /// Returns snapshots filtered by time range, ordered by timestamp ascending.
    pub fn query_stats_history(
        &self,
        queue_name: &str,
        from_epoch: Option<u64>,
        to_epoch: Option<u64>,
        limit: usize,
    ) -> Vec<StoredQueueSnapshot> {
        if let Some(buffer) = self.stats_history.get(queue_name) {
            buffer
                .iter()
                .filter(|s| from_epoch.map_or(true, |from| s.timestamp_epoch >= from))
                .filter(|s| to_epoch.map_or(true, |to| s.timestamp_epoch <= to))
                .take(limit)
                .cloned()
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Spawn a periodic task that publishes queue stats snapshots to the event bus.
    pub fn start_stats_publisher(
        self: &Arc<Self>,
        interval_secs: u64,
        tenant_id: TenantId,
    ) -> tokio::task::JoinHandle<()> {
        let this = Arc::clone(self);
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs(interval_secs));
            loop {
                interval.tick().await;
                let stats = this.all_queue_stats();
                if !stats.is_empty() {
                    events::stats_snapshot(&this.events, stats.clone(), &tenant_id).await;
                    this.record_stats_snapshots(&stats, &tenant_id).await;
                }
                this.check_sla_warnings(&tenant_id).await;
            }
        })
    }

    /// Reset all daily statistics counters to zero (UTC midnight reset).
    pub fn reset_daily_stats(&self) {
        for mut entry in self.calls_handled_today.iter_mut() {
            *entry = 0;
        }
        for mut entry in self.calls_abandoned_today.iter_mut() {
            *entry = 0;
        }
        for mut entry in self.calls_within_sl.iter_mut() {
            *entry = 0;
        }
        for mut entry in self.calls_offered_today.iter_mut() {
            *entry = 0;
        }
        info!("Daily queue stats reset");
    }

    /// Spawn a background task that resets daily stats at UTC midnight.
    pub fn start_daily_reset(self: &Arc<Self>) -> JoinHandle<()> {
        let this = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                let now = std::time::SystemTime::now();
                let since_epoch = now.duration_since(std::time::UNIX_EPOCH).unwrap();
                let secs_today = since_epoch.as_secs() % 86400;
                let secs_until_midnight = 86400 - secs_today;
                tokio::time::sleep(Duration::from_secs(secs_until_midnight)).await;
                this.reset_daily_stats();
            }
        })
    }

    /// Look up a queued call by its SIP Call-ID without removing it.
    pub fn get_queued_call(&self, call_id: &str) -> Option<QueuedCall> {
        for queue in self.queues.iter() {
            if let Some(call) = queue.waiting_calls.iter().find(|c| c.call_id == call_id) {
                return Some(call.clone());
            }
        }
        None
    }

    /// Mark a queued call as a callback request. The call stays in the queue
    /// but will be handled as an outbound callback when an agent becomes available.
    pub fn mark_callback(&self, call_id: &str, callback_uri: &str) -> bool {
        for mut queue in self.queues.iter_mut() {
            if let Some(call) = queue.waiting_calls.iter_mut().find(|c| c.call_id == call_id) {
                call.is_callback = true;
                call.callback_uri = Some(callback_uri.to_string());
                return true;
            }
        }
        false
    }

    /// Get all callback calls in a specific queue.
    /// Returns a list of (call_id, callback_uri) pairs.
    pub fn get_callback_calls(&self, queue_name: &str) -> Vec<(String, String)> {
        if let Some(queue) = self.queues.get(queue_name) {
            queue
                .waiting_calls
                .iter()
                .filter(|c| c.is_callback)
                .filter_map(|c| {
                    c.callback_uri
                        .as_ref()
                        .map(|uri| (c.call_id.clone(), uri.clone()))
                })
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Get callback calls that are ready to be attempted (cooldown expired).
    /// Filters by: is_callback, has callback_uri, and last_callback_attempt
    /// is either None or older than the queue's callback_retry_delay_secs.
    /// Returns (call_id, callback_uri) pairs.
    pub fn get_ready_callbacks(&self, queue_name: &str) -> Vec<(String, String)> {
        if let Some(queue) = self.queues.get(queue_name) {
            let delay = Duration::from_secs(queue.config.callback_retry_delay_secs as u64);
            queue
                .waiting_calls
                .iter()
                .filter(|c| c.is_callback && c.callback_uri.is_some())
                .filter(|c| match c.last_callback_attempt {
                    None => true,
                    Some(last) => last.elapsed() >= delay,
                })
                .filter_map(|c| {
                    c.callback_uri
                        .as_ref()
                        .map(|uri| (c.call_id.clone(), uri.clone()))
                })
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Increment the callback attempt counter for a call and set last_callback_attempt.
    /// Returns the new attempt count, or None if call not found.
    pub fn increment_callback_attempts(&self, call_id: &str) -> Option<u32> {
        for mut queue in self.queues.iter_mut() {
            if let Some(call) = queue.waiting_calls.iter_mut().find(|c| c.call_id == call_id) {
                call.callback_attempts += 1;
                call.last_callback_attempt = Some(Instant::now());
                return Some(call.callback_attempts);
            }
        }
        None
    }

    /// Cancel a pending callback by removing it from the queue.
    /// Returns true if the callback was found and removed.
    pub fn cancel_callback(&self, call_id: &str) -> bool {
        for mut queue in self.queues.iter_mut() {
            if let Some(idx) = queue
                .waiting_calls
                .iter()
                .position(|c| c.call_id == call_id && c.is_callback)
            {
                queue.waiting_calls.remove(idx);
                queue.recompute_positions();
                info!(call_id = %call_id, queue = %queue.name, "Callback cancelled");
                return true;
            }
        }
        false
    }

    /// List all pending callbacks across all queues for the REST API.
    pub fn list_all_callbacks(&self) -> Vec<crate::types::CallbackInfo> {
        let now = Instant::now();
        let mut result = Vec::new();
        for queue in self.queues.iter() {
            for call in &queue.waiting_calls {
                if call.is_callback {
                    if let Some(ref uri) = call.callback_uri {
                        result.push(crate::types::CallbackInfo {
                            call_id: call.call_id.clone(),
                            queue_name: queue.name.clone(),
                            callback_uri: uri.clone(),
                            caller_display: call.caller_display.clone(),
                            attempts: call.callback_attempts,
                            enqueued_at_secs_ago: now
                                .duration_since(call.enqueued_at)
                                .as_secs(),
                            last_attempt_secs_ago: call
                                .last_callback_attempt
                                .map(|t| now.duration_since(t).as_secs()),
                        });
                    }
                }
            }
        }
        result
    }

    /// Remove callbacks that have exceeded callback_max_attempts.
    /// Returns removed calls for event publishing.
    pub fn remove_expired_callbacks(&self, queue_name: &str) -> Vec<QueuedCall> {
        let mut expired = Vec::new();
        if let Some(mut queue) = self.queues.get_mut(queue_name) {
            let max_attempts = queue.config.callback_max_attempts;
            if max_attempts == 0 {
                return expired; // Unlimited attempts
            }
            let mut i = 0;
            while i < queue.waiting_calls.len() {
                if queue.waiting_calls[i].is_callback
                    && queue.waiting_calls[i].callback_attempts >= max_attempts
                {
                    let call = queue.waiting_calls.remove(i);
                    info!(
                        call_id = %call.call_id,
                        queue = %queue_name,
                        attempts = call.callback_attempts,
                        max = max_attempts,
                        "Callback expired after max attempts"
                    );
                    expired.push(call);
                } else {
                    i += 1;
                }
            }
            if !expired.is_empty() {
                queue.recompute_positions();
            }
        }
        expired
    }

    /// Handle callback recipient failure (no-answer/reject/timeout).
    /// Unlike agent_no_answer(), this does NOT penalize the agent's no-answer counter.
    /// Instead, it makes the agent available again and increments callback attempts.
    pub async fn agent_callback_recipient_failed(&self, agent_id: &str, call_id: &str) {
        // Make agent available without penalizing
        if let Some(mut agent) = self.agents.get_mut(agent_id) {
            agent.current_calls = agent.current_calls.saturating_sub(1);
            if agent.current_calls == 0 {
                agent.state = AgentState::Available;
                agent.state_changed_at = Instant::now();
            }
        }

        // Increment callback attempt counter
        self.increment_callback_attempts(call_id);
    }

    /// Restore agent state from persistent storage after a service restart.
    /// Returns the number of agents restored.
    pub async fn restore_agents(
        &self,
        store: &dyn QueueStore,
        tenant_id: &TenantId,
    ) -> Result<usize> {
        let stored = store.load_agents(tenant_id).await?;
        let count = stored.len();

        for sa in stored {
            // Only restore agents whose queues still exist in config
            let valid_queues: Vec<String> = sa
                .queues
                .iter()
                .filter(|q| self.queues.contains_key(*q))
                .cloned()
                .collect();

            if valid_queues.is_empty() {
                debug!(agent_id = %sa.id, "Skipping agent restore: no valid queues");
                continue;
            }

            let state = match sa.state.as_str() {
                "available" => AgentState::Available,
                "away" => AgentState::Away,
                "paused" => AgentState::Paused,
                _ => {
                    debug!(agent_id = %sa.id, state = %sa.state, "Skipping agent with non-active state");
                    continue;
                }
            };

            let mut agent = Agent::new(sa.id.clone(), sa.extension.clone(), tenant_id.clone());
            agent.queues = valid_queues;
            agent.skills = sa.skills;
            agent.priority = sa.priority;
            agent.queue_priorities = sa.queue_priorities;
            agent.state = state;
            agent.state_changed_at = Instant::now();
            agent.logged_in_at = Some(Instant::now());
            agent.max_concurrent_calls = sa.max_concurrent_calls;

            info!(agent_id = %sa.id, "Restored agent from storage");
            self.agents.insert(sa.id, agent);
        }

        Ok(count)
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
                agent_weights: HashMap::new(),
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
            },
        );
        QueueSystemConfig {
            enabled: true,
            queues,
            wrap_up_time_secs: 5,
            service_level_threshold_secs: 20,
            max_ring_time_secs: 20,
            queue_audio_dir: None,
            stats_publish_interval_secs: 5,
            auto_pause_resume_secs: 0,
            max_no_answer: 3,
            default_max_concurrent_calls: 1,
            recording_dir: None,
            webhook_url: None,
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
            is_callback: false,
            callback_uri: None,
            original_priority: 0,
            callback_attempts: 0,
            last_callback_attempt: None,
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
            .agent_login("100", "100", &["sales".to_string()], tenant.clone(), Vec::new())
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
            .agent_login("100", "100", &["nonexistent".to_string()], test_tenant(), Vec::new())
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
            .agent_login("100", "100", &["sales".to_string()], tenant.clone(), Vec::new())
            .await
            .unwrap();
        manager
            .agent_login("101", "101", &["sales".to_string()], tenant.clone(), Vec::new())
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
            .agent_login("100", "100", &["sales".to_string()], tenant.clone(), Vec::new())
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
            .agent_login("100", "100", &["sales".to_string()], tenant, Vec::new())
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
