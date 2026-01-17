//! Call routing engine.
//!
//! This module provides the core routing logic for the FCP platform, including:
//! - Inbound call routing (E.164 → LineBinding → Devices)
//! - Extension resolution
//! - Ring strategy implementation
//! - Call forking logic

use crate::{
    DeviceId, ExtensionTarget, LineBinding, PhoneNumber, PhoneNumberId, RingStrategy, TenantId,
    UserId,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Result type for routing operations.
pub type Result<T> = std::result::Result<T, RoutingError>;

/// Routing errors.
#[derive(Debug, thiserror::Error)]
pub enum RoutingError {
    #[error("No route found for destination: {0}")]
    NoRoute(String),

    #[error("Extension not found: {0}")]
    ExtensionNotFound(String),

    #[error("Phone number not found: {0}")]
    PhoneNumberNotFound(String),

    #[error("Phone number not routable: {0}")]
    PhoneNumberNotRoutable(String),

    #[error("Line binding not found for phone number: {0}")]
    LineBindingNotFound(String),

    #[error("No available devices for routing")]
    NoDevicesAvailable,

    #[error("Invalid destination: {0}")]
    InvalidDestination(String),

    #[error("Storage error: {0}")]
    Storage(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

/// A routing target represents a destination for a call.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RoutingTarget {
    /// The device ID to route to
    pub device_id: DeviceId,

    /// The SIP URI to contact
    pub sip_uri: String,

    /// Priority for this target (lower = higher priority)
    pub priority: u8,

    /// Line appearance number
    pub line_appearance: u8,

    /// Whether this is a shared line
    pub shared_line: bool,

    /// Whether this target is enabled
    pub enabled: bool,

    /// Timeout for this target
    pub timeout: Duration,
}

/// The result of a routing decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingResult {
    /// The resolved phone number (if applicable)
    pub phone_number: Option<PhoneNumber>,

    /// The line binding used for routing
    pub line_binding: Option<LineBinding>,

    /// The list of routing targets
    pub targets: Vec<RoutingTarget>,

    /// The ring strategy to use
    pub ring_strategy: RingStrategy,

    /// Call forwarding destination (if applicable)
    pub forward_to: Option<String>,

    /// No-answer timeout
    pub no_answer_timeout: Duration,
}

impl RoutingResult {
    /// Returns true if this result has any valid targets.
    pub fn has_targets(&self) -> bool {
        !self.targets.is_empty() && self.targets.iter().any(|t| t.enabled)
    }

    /// Returns enabled targets sorted by priority.
    pub fn enabled_targets(&self) -> Vec<&RoutingTarget> {
        let mut targets: Vec<&RoutingTarget> = self
            .targets
            .iter()
            .filter(|t| t.enabled)
            .collect();
        targets.sort_by_key(|t| t.priority);
        targets
    }

    /// Returns targets grouped by priority for simultaneous ringing.
    pub fn targets_by_priority(&self) -> Vec<Vec<&RoutingTarget>> {
        let mut result: Vec<Vec<&RoutingTarget>> = Vec::new();
        let mut enabled = self.enabled_targets();

        while !enabled.is_empty() {
            let current_priority = enabled[0].priority;
            let same_priority: Vec<&RoutingTarget> = enabled
                .iter()
                .copied()
                .take_while(|t| t.priority == current_priority)
                .collect();

            let count = same_priority.len();
            result.push(same_priority);
            enabled.drain(..count);
        }

        result
    }
}

/// Routing destination - what the caller dialed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum RoutingDestination {
    /// E.164 phone number (e.g., "+15551234567")
    E164(String),

    /// Extension number (e.g., "1001")
    Extension(String),

    /// SIP URI (e.g., "alice@example.com")
    SipUri(String),

    /// User ID (for internal routing)
    UserId(UserId),
}

impl RoutingDestination {
    /// Creates a routing destination from a string, auto-detecting the type.
    pub fn from_string(s: &str) -> Self {
        if s.starts_with('+') && s[1..].chars().all(|c| c.is_ascii_digit()) {
            RoutingDestination::E164(s.to_string())
        } else if s.contains('@') {
            RoutingDestination::SipUri(s.to_string())
        } else if s.chars().all(|c| c.is_ascii_digit()) {
            RoutingDestination::Extension(s.to_string())
        } else {
            RoutingDestination::SipUri(s.to_string())
        }
    }
}

impl std::fmt::Display for RoutingDestination {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RoutingDestination::E164(num) => write!(f, "{}", num),
            RoutingDestination::Extension(ext) => write!(f, "{}", ext),
            RoutingDestination::SipUri(uri) => write!(f, "{}", uri),
            RoutingDestination::UserId(id) => write!(f, "user:{}", id),
        }
    }
}

/// The routing engine trait.
#[async_trait]
pub trait RoutingEngine: Send + Sync {
    /// Route an inbound call to its destination.
    ///
    /// This is the main routing entry point. It:
    /// 1. Determines the destination type (E.164, extension, SIP URI)
    /// 2. Looks up the appropriate line binding(s)
    /// 3. Applies ring strategy
    /// 4. Returns a list of targets to fork to
    async fn route_inbound_call(
        &self,
        destination: &RoutingDestination,
        tenant_id: &TenantId,
        caller: &str,
    ) -> Result<RoutingResult>;

    /// Resolve an extension to its target.
    async fn resolve_extension(
        &self,
        extension: &str,
        tenant_id: &TenantId,
    ) -> Result<ExtensionTarget>;

    /// Route to a specific user ID.
    async fn route_to_user(
        &self,
        user_id: &UserId,
        tenant_id: &TenantId,
        caller: &str,
    ) -> Result<RoutingResult>;

    /// Route to a specific phone number.
    async fn route_to_phone_number(
        &self,
        phone_number_id: &PhoneNumberId,
        tenant_id: &TenantId,
        caller: &str,
    ) -> Result<RoutingResult>;

    /// Check if a destination should be forwarded.
    async fn check_call_forward(
        &self,
        line_binding: &LineBinding,
        tenant_id: &TenantId,
        caller: &str,
    ) -> Option<String>;
}

/// Ring group represents a set of devices that should ring together.
#[derive(Debug, Clone)]
pub struct RingGroup {
    /// The devices in this group
    pub targets: Vec<RoutingTarget>,

    /// How long to wait before trying the next group
    pub timeout: Duration,
}

/// Ring plan represents the complete routing plan for a call.
#[derive(Debug, Clone)]
pub struct RingPlan {
    /// The ring groups, in order
    pub groups: Vec<RingGroup>,

    /// The overall ring strategy
    pub strategy: RingStrategy,

    /// Total timeout before no-answer
    pub total_timeout: Duration,
}

impl RingPlan {
    /// Creates a ring plan from a routing result.
    pub fn from_routing_result(result: &RoutingResult) -> Self {
        let groups = match result.ring_strategy {
            RingStrategy::Simultaneous => {
                // All devices ring at once
                vec![RingGroup {
                    targets: result.targets.clone(),
                    timeout: result.no_answer_timeout,
                }]
            }
            RingStrategy::Sequential => {
                // Each device rings one at a time
                result
                    .enabled_targets()
                    .iter()
                    .map(|target| RingGroup {
                        targets: vec![(*target).clone()],
                        timeout: target.timeout,
                    })
                    .collect()
            }
            RingStrategy::RoundRobin => {
                // Similar to sequential but tracks last used
                // For now, treat same as sequential (state tracking would be added later)
                result
                    .enabled_targets()
                    .iter()
                    .map(|target| RingGroup {
                        targets: vec![(*target).clone()],
                        timeout: target.timeout,
                    })
                    .collect()
            }
            RingStrategy::LongestIdle => {
                // Would need to track device idle time
                // For now, treat same as sequential
                result
                    .enabled_targets()
                    .iter()
                    .map(|target| RingGroup {
                        targets: vec![(*target).clone()],
                        timeout: target.timeout,
                    })
                    .collect()
            }
            RingStrategy::Primary => {
                // Ring by priority groups
                result
                    .targets_by_priority()
                    .into_iter()
                    .map(|priority_group| {
                        let group_len = priority_group.len();
                        RingGroup {
                            targets: priority_group.into_iter().cloned().collect(),
                            timeout: result.no_answer_timeout / group_len as u32,
                        }
                    })
                    .collect()
            }
        };

        Self {
            groups,
            strategy: result.ring_strategy,
            total_timeout: result.no_answer_timeout,
        }
    }

    /// Returns the total number of targets across all groups.
    pub fn total_targets(&self) -> usize {
        self.groups.iter().map(|g| g.targets.len()).sum()
    }

    /// Returns true if this plan has any targets.
    pub fn has_targets(&self) -> bool {
        self.total_targets() > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_routing_destination_from_string() {
        assert_eq!(
            RoutingDestination::from_string("+15551234567"),
            RoutingDestination::E164("+15551234567".to_string())
        );

        assert_eq!(
            RoutingDestination::from_string("1001"),
            RoutingDestination::Extension("1001".to_string())
        );

        assert_eq!(
            RoutingDestination::from_string("alice@example.com"),
            RoutingDestination::SipUri("alice@example.com".to_string())
        );
    }

    #[test]
    fn test_routing_result_enabled_targets() {
        let result = RoutingResult {
            phone_number: None,
            line_binding: None,
            targets: vec![
                RoutingTarget {
                    device_id: DeviceId::new(),
                    sip_uri: "sip:device1@example.com".to_string(),
                    priority: 2,
                    line_appearance: 1,
                    shared_line: false,
                    enabled: true,
                    timeout: Duration::from_secs(30),
                },
                RoutingTarget {
                    device_id: DeviceId::new(),
                    sip_uri: "sip:device2@example.com".to_string(),
                    priority: 1,
                    line_appearance: 1,
                    shared_line: false,
                    enabled: false,
                    timeout: Duration::from_secs(30),
                },
                RoutingTarget {
                    device_id: DeviceId::new(),
                    sip_uri: "sip:device3@example.com".to_string(),
                    priority: 1,
                    line_appearance: 1,
                    shared_line: false,
                    enabled: true,
                    timeout: Duration::from_secs(30),
                },
            ],
            ring_strategy: RingStrategy::Simultaneous,
            forward_to: None,
            no_answer_timeout: Duration::from_secs(30),
        };

        let enabled = result.enabled_targets();
        assert_eq!(enabled.len(), 2);
        assert_eq!(enabled[0].priority, 1); // Sorted by priority
        assert_eq!(enabled[1].priority, 2);
    }

    #[test]
    fn test_ring_plan_simultaneous() {
        let result = RoutingResult {
            phone_number: None,
            line_binding: None,
            targets: vec![
                RoutingTarget {
                    device_id: DeviceId::new(),
                    sip_uri: "sip:device1@example.com".to_string(),
                    priority: 1,
                    line_appearance: 1,
                    shared_line: false,
                    enabled: true,
                    timeout: Duration::from_secs(30),
                },
                RoutingTarget {
                    device_id: DeviceId::new(),
                    sip_uri: "sip:device2@example.com".to_string(),
                    priority: 1,
                    line_appearance: 1,
                    shared_line: false,
                    enabled: true,
                    timeout: Duration::from_secs(30),
                },
            ],
            ring_strategy: RingStrategy::Simultaneous,
            forward_to: None,
            no_answer_timeout: Duration::from_secs(60),
        };

        let plan = RingPlan::from_routing_result(&result);
        assert_eq!(plan.groups.len(), 1);
        assert_eq!(plan.groups[0].targets.len(), 2);
        assert_eq!(plan.total_targets(), 2);
    }

    #[test]
    fn test_ring_plan_sequential() {
        let result = RoutingResult {
            phone_number: None,
            line_binding: None,
            targets: vec![
                RoutingTarget {
                    device_id: DeviceId::new(),
                    sip_uri: "sip:device1@example.com".to_string(),
                    priority: 1,
                    line_appearance: 1,
                    shared_line: false,
                    enabled: true,
                    timeout: Duration::from_secs(20),
                },
                RoutingTarget {
                    device_id: DeviceId::new(),
                    sip_uri: "sip:device2@example.com".to_string(),
                    priority: 2,
                    line_appearance: 1,
                    shared_line: false,
                    enabled: true,
                    timeout: Duration::from_secs(20),
                },
            ],
            ring_strategy: RingStrategy::Sequential,
            forward_to: None,
            no_answer_timeout: Duration::from_secs(60),
        };

        let plan = RingPlan::from_routing_result(&result);
        assert_eq!(plan.groups.len(), 2);
        assert_eq!(plan.groups[0].targets.len(), 1);
        assert_eq!(plan.groups[1].targets.len(), 1);
        assert_eq!(plan.total_targets(), 2);
    }
}
