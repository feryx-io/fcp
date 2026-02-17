//! ACD/Queue service for automatic call distribution.
//!
//! This crate provides the core types, configuration, and runtime state for
//! an ACD (Automatic Call Distribution) queue system. Unlike hunt groups (which
//! ring members and give up if nobody answers), ACD queues hold callers in a
//! waiting queue with MOH, track agent availability, and distribute calls using
//! intelligent strategies.
//!
//! # Architecture
//!
//! The queue system is a **library crate** that provides:
//! - [`QueueManager`]: Runtime state for queues, agents, and call distribution
//! - [`QueueSystemConfig`]: TOML-deserializable configuration
//! - [`DistributionPlan`]: Output of the distribution engine (which agent to ring)
//!
//! The concrete SIP handler (`QueueCallHandler`) that implements `QueueHandler`
//! lives in the `fcp-call-manager` binary and wires together the queue manager
//! with RTP/MOH and SIP signaling.

pub mod config;
pub mod distribution;
pub mod events;
pub mod queue_manager;
pub mod types;

// Re-exports
pub use config::{DistributionStrategy, QueueConfig, QueueSystemConfig};
pub use distribution::DistributionPlan;
pub use queue_manager::QueueManager;
pub use types::{Agent, AgentState, AgentStats, Queue, QueueStats, QueuedCall};
