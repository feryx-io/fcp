//! Data models for the routing system.
//!
//! This module contains all data structures used by the routing engine:
//! - Identity models (users, roles, groups)
//! - Policy models (rules, conditions, actions)
//! - Reachability models (destinations, classifications) [FUTURE]
//! - Routing models (trunks, routes) [FUTURE]
//! - Transform models (header/number manipulation) [FUTURE]

pub mod identity;
pub mod policy;

// Re-export key types
pub use identity::{DeviceType, Endpoint, IdentityContext, User};
pub use policy::{
    AppliesTo, DestinationMatch, IpRange, PolicyAction, PolicyDecision, PolicyRule, TimeWindow,
};
