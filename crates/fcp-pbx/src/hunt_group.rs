//! Hunt group ring strategy resolution.
//!
//! Produces a `HuntGroupPlan` describing which members to ring, in what order,
//! and with what forking configuration. The caller (SipMessageHandler) executes
//! the plan using its existing B-leg infrastructure.

use crate::config::{HuntGroupConfig, RingStrategy};
use fcp_b2bua_core::forking::ForkingConfig;
use rand::seq::SliceRandom;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A resolved hunt group ready for execution.
pub struct HuntGroup {
    /// Extension number (e.g., "5000").
    pub extension: String,

    /// Underlying configuration.
    pub config: HuntGroupConfig,

    /// Round-robin rotation counter (shared across calls).
    round_robin_counter: AtomicUsize,
}

impl HuntGroup {
    /// Create a new hunt group from configuration.
    pub fn new(extension: String, config: HuntGroupConfig) -> Self {
        Self {
            extension,
            config,
            round_robin_counter: AtomicUsize::new(0),
        }
    }

    /// Resolve the ring strategy into a concrete plan.
    pub fn resolve(&self) -> HuntGroupPlan {
        match self.config.strategy {
            RingStrategy::Simultaneous => self.resolve_simultaneous(),
            RingStrategy::Sequential => self.resolve_sequential(),
            RingStrategy::RoundRobin => self.resolve_round_robin(),
            RingStrategy::Random => self.resolve_random(),
        }
    }

    fn resolve_simultaneous(&self) -> HuntGroupPlan {
        HuntGroupPlan {
            ring_phases: vec![RingPhase {
                members: self.config.members.clone(),
                timeout_secs: self.config.ring_timeout,
                simultaneous: true,
            }],
            forking_config: ForkingConfig::pbx_parallel(),
            no_answer_destination: self.config.no_answer_destination.clone(),
            total_timeout_secs: self.config.total_timeout,
        }
    }

    fn resolve_sequential(&self) -> HuntGroupPlan {
        let phases = self
            .config
            .members
            .iter()
            .map(|member| RingPhase {
                members: vec![member.clone()],
                timeout_secs: self.config.ring_timeout,
                simultaneous: false,
            })
            .collect();

        HuntGroupPlan {
            ring_phases: phases,
            forking_config: ForkingConfig::pbx_sequential(),
            no_answer_destination: self.config.no_answer_destination.clone(),
            total_timeout_secs: self.config.total_timeout,
        }
    }

    fn resolve_round_robin(&self) -> HuntGroupPlan {
        let len = self.config.members.len();
        if len == 0 {
            return HuntGroupPlan {
                ring_phases: vec![],
                forking_config: ForkingConfig::pbx_sequential(),
                no_answer_destination: self.config.no_answer_destination.clone(),
                total_timeout_secs: self.config.total_timeout,
            };
        }

        let start = self.round_robin_counter.fetch_add(1, Ordering::Relaxed) % len;

        let rotated: Vec<String> = (0..len)
            .map(|i| self.config.members[(start + i) % len].clone())
            .collect();

        let phases = rotated
            .into_iter()
            .map(|member| RingPhase {
                members: vec![member],
                timeout_secs: self.config.ring_timeout,
                simultaneous: false,
            })
            .collect();

        HuntGroupPlan {
            ring_phases: phases,
            forking_config: ForkingConfig::pbx_sequential(),
            no_answer_destination: self.config.no_answer_destination.clone(),
            total_timeout_secs: self.config.total_timeout,
        }
    }

    fn resolve_random(&self) -> HuntGroupPlan {
        let mut members = self.config.members.clone();
        let mut rng = rand::thread_rng();
        members.shuffle(&mut rng);

        HuntGroupPlan {
            ring_phases: vec![RingPhase {
                members,
                timeout_secs: self.config.ring_timeout,
                simultaneous: true,
            }],
            forking_config: ForkingConfig::pbx_parallel(),
            no_answer_destination: self.config.no_answer_destination.clone(),
            total_timeout_secs: self.config.total_timeout,
        }
    }
}

/// Execution plan produced by a hunt group resolution.
#[derive(Debug, Clone)]
pub struct HuntGroupPlan {
    /// Ordered ring phases to execute.
    pub ring_phases: Vec<RingPhase>,

    /// Forking configuration (parallel vs sequential).
    pub forking_config: ForkingConfig,

    /// Where to route if nobody answers (e.g., "voicemail:5000").
    pub no_answer_destination: Option<String>,

    /// Maximum total time across all phases.
    pub total_timeout_secs: u32,
}

/// A single phase of ringing within a hunt group plan.
#[derive(Debug, Clone)]
pub struct RingPhase {
    /// Extensions to ring in this phase.
    pub members: Vec<String>,

    /// How long to ring before moving to the next phase.
    pub timeout_secs: u32,

    /// Whether all members ring simultaneously in this phase.
    pub simultaneous: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::HuntGroupConfig;

    fn test_config(strategy: RingStrategy) -> HuntGroupConfig {
        HuntGroupConfig {
            name: "Test".to_string(),
            strategy,
            members: vec!["1001".to_string(), "1002".to_string(), "1003".to_string()],
            ring_timeout: 20,
            total_timeout: 60,
            no_answer_destination: Some("voicemail:5000".to_string()),
            skip_busy: false,
        }
    }

    #[test]
    fn test_simultaneous_plan() {
        let hg = HuntGroup::new("5000".to_string(), test_config(RingStrategy::Simultaneous));
        let plan = hg.resolve();
        assert_eq!(plan.ring_phases.len(), 1);
        assert!(plan.ring_phases[0].simultaneous);
        assert_eq!(plan.ring_phases[0].members.len(), 3);
        assert_eq!(
            plan.forking_config.mode,
            fcp_b2bua_core::forking::ForkingMode::Parallel
        );
    }

    #[test]
    fn test_sequential_plan() {
        let hg = HuntGroup::new("5000".to_string(), test_config(RingStrategy::Sequential));
        let plan = hg.resolve();
        assert_eq!(plan.ring_phases.len(), 3);
        assert!(!plan.ring_phases[0].simultaneous);
        assert_eq!(plan.ring_phases[0].members, vec!["1001"]);
        assert_eq!(plan.ring_phases[1].members, vec!["1002"]);
        assert_eq!(plan.ring_phases[2].members, vec!["1003"]);
    }

    #[test]
    fn test_round_robin_rotates() {
        let hg = HuntGroup::new("5000".to_string(), test_config(RingStrategy::RoundRobin));

        let plan1 = hg.resolve();
        let plan2 = hg.resolve();

        // First call starts at index 0, second at index 1
        assert_eq!(plan1.ring_phases[0].members[0], "1001");
        assert_eq!(plan2.ring_phases[0].members[0], "1002");
    }

    #[test]
    fn test_random_plan_all_members() {
        let hg = HuntGroup::new("5000".to_string(), test_config(RingStrategy::Random));
        let plan = hg.resolve();
        assert_eq!(plan.ring_phases.len(), 1);
        assert!(plan.ring_phases[0].simultaneous);
        assert_eq!(plan.ring_phases[0].members.len(), 3);
        // All members should be present (just possibly reordered)
        let mut sorted = plan.ring_phases[0].members.clone();
        sorted.sort();
        assert_eq!(sorted, vec!["1001", "1002", "1003"]);
    }

    #[test]
    fn test_no_answer_destination() {
        let hg = HuntGroup::new("5000".to_string(), test_config(RingStrategy::Simultaneous));
        let plan = hg.resolve();
        assert_eq!(
            plan.no_answer_destination,
            Some("voicemail:5000".to_string())
        );
    }

    #[test]
    fn test_empty_members() {
        let mut cfg = test_config(RingStrategy::RoundRobin);
        cfg.members.clear();
        let hg = HuntGroup::new("5000".to_string(), cfg);
        let plan = hg.resolve();
        assert!(plan.ring_phases.is_empty());
    }
}
