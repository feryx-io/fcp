//! SBC (Session Border Controller) configuration and shared trunk runtime library.
//!
//! Provides config types for all SBC subsystems: call admission control,
//! rate limiting, security (IP ACLs, malformed SIP policy), trunk/carrier
//! definitions, NAT traversal, SIPREC recording, and metrics.
//!
//! Also provides shared trunk runtime infrastructure (health monitoring,
//! authentication, registration, routing, health probes) used by both
//! fcp-sbc and fcp-call-manager (session-manager mode).

// Configuration modules
pub mod admission;
pub mod config;
pub mod hmr;
pub mod metrics;
pub mod nat;
pub mod rate_limit;
pub mod security;
pub mod siprec;
pub mod stir_shaken;
pub mod trunk;

// Shared trunk runtime modules
pub mod cps;
pub mod health_probes;
pub mod lcr;
pub mod trunk_auth;
pub mod trunk_health;
pub mod trunk_quality;
pub mod trunk_registration;
pub mod trunk_router;

// Re-export config types for convenience
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
pub use stir_shaken::{
    AttestationLevel, StirShakenConfig, StirSignMode, StirVerifyMode,
    TrunkStirShakenConfig,
};
pub use trunk::{SrtpMode, TrunkConfig, TrunkCredentials, TrunkDirection, TrunkDtmfConfig, TrunkDtmfMode, TrunkFailover, TrunkSrtpConfig, TrunkTransport};

// Re-export LCR and quality types
pub use lcr::{LcrConfig, LcrRouteEntry, LoadBalanceStrategy};
pub use trunk_quality::{TrunkQualitySnapshot, TrunkQualityTracker};

// Re-export trunk runtime types for convenience
pub use cps::{CpsChecker, NoCpsLimit};
pub use trunk_auth::TrunkCredentialProvider;
pub use trunk_health::TrunkHealthMonitor;
pub use trunk_registration::{TrunkRegistrar, RegistrationState, run_trunk_registration};
pub use trunk_router::TrunkRouter;
pub use health_probes::run_health_probes;
