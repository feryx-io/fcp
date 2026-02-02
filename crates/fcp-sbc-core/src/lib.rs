//! SBC (Session Border Controller) policy-level configuration library.
//!
//! Provides config types for all SBC subsystems: call admission control,
//! rate limiting, security (IP ACLs, malformed SIP policy), trunk/carrier
//! definitions, NAT traversal, SIPREC recording, and metrics.
//!
//! This crate contains no async code or runtime state -- it is purely
//! configuration types with `Serialize`/`Deserialize`, `Default` impls,
//! and `validate()` methods.

pub mod admission;
pub mod config;
pub mod hmr;
pub mod metrics;
pub mod nat;
pub mod rate_limit;
pub mod security;
pub mod siprec;
pub mod trunk;

// Re-export key types for convenience
pub use admission::{AdmissionAction, AdmissionConfig, TrunkCpsLimit};
pub use config::{SbcConfig, SbcStorageConfig};
pub use metrics::{SbcMetricsConfig, SbcMetricsSnapshot, TrunkHealthStatus};
pub use nat::{NatConfig, StunServerConfig, TurnServerConfig};
pub use rate_limit::{MethodRateLimits, RateLimitEntry, RateLimitPolicyConfig};
pub use security::{AclAction, IpAcl, IpAclEntry, MalformedSipPolicy, SecurityConfig};
pub use siprec::{RecordingPolicy, RecordingServer, SelectiveRecordingRule, SiprecConfig};
pub use hmr::{
    HeaderManipulationConfig, HmrAction, HmrActionType, HmrCondition, HmrDirection,
    HmrMessageType, HmrRule,
};
pub use trunk::{TrunkConfig, TrunkCredentials, TrunkDirection, TrunkFailover, TrunkTransport};
