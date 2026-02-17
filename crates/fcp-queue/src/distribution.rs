//! Distribution engine for selecting agents based on queue strategy.

use crate::config::DistributionStrategy;
use crate::types::Agent;

/// A plan describing which agent to ring for a queued call.
#[derive(Debug, Clone)]
pub struct DistributionPlan {
    /// The selected agent's SIP extension.
    pub agent_extension: String,
    /// The selected agent's ID.
    pub agent_id: String,
    /// Ring timeout in seconds for this attempt.
    pub ring_timeout_secs: u32,
}

/// Select an agent from the given list of available agents based on the strategy.
///
/// `round_robin_index` is the current round-robin counter for the queue (only used
/// for `RoundRobin` strategy). Returns the selected agent index and updated counter.
///
/// Returns `None` if no agents are available.
pub fn select_agent(
    strategy: DistributionStrategy,
    available_agents: &[&Agent],
    round_robin_index: usize,
    ring_timeout_secs: u32,
) -> Option<(DistributionPlan, usize)> {
    if available_agents.is_empty() {
        return None;
    }

    let (agent, new_rr_index) = match strategy {
        DistributionStrategy::RoundRobin => {
            let idx = round_robin_index % available_agents.len();
            (available_agents[idx], round_robin_index + 1)
        }

        DistributionStrategy::LongestIdle => {
            let agent = available_agents
                .iter()
                .min_by_key(|a| a.last_call_ended.map(|t| t.elapsed()))
                .copied()?;
            // For longest idle: the agent with the oldest last_call_ended wins.
            // If last_call_ended is None, the agent has never taken a call — treat as longest idle.
            let agent = available_agents
                .iter()
                .max_by(|a, b| {
                    let a_idle = a
                        .last_call_ended
                        .map(|t| t.elapsed())
                        .unwrap_or(std::time::Duration::MAX);
                    let b_idle = b
                        .last_call_ended
                        .map(|t| t.elapsed())
                        .unwrap_or(std::time::Duration::MAX);
                    a_idle.cmp(&b_idle)
                })
                .copied()
                .unwrap_or(agent);
            (agent, round_robin_index)
        }

        DistributionStrategy::FewestCalls => {
            let agent = available_agents
                .iter()
                .min_by_key(|a| a.calls_handled)
                .copied()?;
            (agent, round_robin_index)
        }

        DistributionStrategy::Random => {
            use rand::Rng;
            let idx = rand::thread_rng().gen_range(0..available_agents.len());
            (available_agents[idx], round_robin_index)
        }

        DistributionStrategy::Linear => {
            // Agents should be ordered by their config order; pick the first available.
            (available_agents[0], round_robin_index)
        }
    };

    Some((
        DistributionPlan {
            agent_extension: agent.extension.clone(),
            agent_id: agent.id.clone(),
            ring_timeout_secs,
        },
        new_rr_index,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Agent, AgentState};
    use fcp_core::TenantId;
    use std::time::{Duration, Instant};

    fn make_agent(id: &str, calls_handled: u64, idle_ago: Option<Duration>) -> Agent {
        let mut agent = Agent::new(id.to_string(), id.to_string(), TenantId::new());
        agent.state = AgentState::Available;
        agent.calls_handled = calls_handled;
        agent.last_call_ended = idle_ago.map(|d| Instant::now() - d);
        agent
    }

    #[test]
    fn test_round_robin() {
        let agents = vec![
            make_agent("100", 0, None),
            make_agent("101", 0, None),
            make_agent("102", 0, None),
        ];
        let refs: Vec<&Agent> = agents.iter().collect();

        let (plan, idx) = select_agent(DistributionStrategy::RoundRobin, &refs, 0, 20).unwrap();
        assert_eq!(plan.agent_id, "100");
        assert_eq!(idx, 1);

        let (plan, idx) = select_agent(DistributionStrategy::RoundRobin, &refs, idx, 20).unwrap();
        assert_eq!(plan.agent_id, "101");
        assert_eq!(idx, 2);

        let (plan, idx) = select_agent(DistributionStrategy::RoundRobin, &refs, idx, 20).unwrap();
        assert_eq!(plan.agent_id, "102");
        assert_eq!(idx, 3);

        // Wraps around
        let (plan, _) = select_agent(DistributionStrategy::RoundRobin, &refs, idx, 20).unwrap();
        assert_eq!(plan.agent_id, "100");
    }

    #[test]
    fn test_fewest_calls() {
        let agents = vec![
            make_agent("100", 10, None),
            make_agent("101", 3, None),
            make_agent("102", 7, None),
        ];
        let refs: Vec<&Agent> = agents.iter().collect();

        let (plan, _) = select_agent(DistributionStrategy::FewestCalls, &refs, 0, 20).unwrap();
        assert_eq!(plan.agent_id, "101");
    }

    #[test]
    fn test_longest_idle() {
        let agents = vec![
            make_agent("100", 0, Some(Duration::from_secs(10))),
            make_agent("101", 0, Some(Duration::from_secs(60))),
            make_agent("102", 0, Some(Duration::from_secs(30))),
        ];
        let refs: Vec<&Agent> = agents.iter().collect();

        let (plan, _) = select_agent(DistributionStrategy::LongestIdle, &refs, 0, 20).unwrap();
        assert_eq!(plan.agent_id, "101");
    }

    #[test]
    fn test_longest_idle_never_taken_call() {
        let agents = vec![
            make_agent("100", 0, Some(Duration::from_secs(60))),
            make_agent("101", 0, None), // Never taken a call = longest idle
        ];
        let refs: Vec<&Agent> = agents.iter().collect();

        let (plan, _) = select_agent(DistributionStrategy::LongestIdle, &refs, 0, 20).unwrap();
        assert_eq!(plan.agent_id, "101");
    }

    #[test]
    fn test_linear() {
        let agents = vec![
            make_agent("100", 5, None),
            make_agent("101", 0, None),
            make_agent("102", 0, None),
        ];
        let refs: Vec<&Agent> = agents.iter().collect();

        // Always picks the first available
        let (plan, _) = select_agent(DistributionStrategy::Linear, &refs, 0, 20).unwrap();
        assert_eq!(plan.agent_id, "100");
    }

    #[test]
    fn test_no_available_agents() {
        let refs: Vec<&Agent> = vec![];
        let result = select_agent(DistributionStrategy::RoundRobin, &refs, 0, 20);
        assert!(result.is_none());
    }

    #[test]
    fn test_ring_timeout_propagated() {
        let agents = vec![make_agent("100", 0, None)];
        let refs: Vec<&Agent> = agents.iter().collect();

        let (plan, _) = select_agent(DistributionStrategy::Linear, &refs, 0, 30).unwrap();
        assert_eq!(plan.ring_timeout_secs, 30);
    }
}
