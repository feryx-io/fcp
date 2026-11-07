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
pub mod hmr_engine;
pub mod metrics;
pub mod nat;
pub mod number_transform;
pub mod rate_limit;
pub mod security;
pub mod siprec;
pub mod stir_shaken;
pub mod trunk;

// Shared trunk runtime modules
pub mod cps;
pub mod drain;
pub mod health_probes;
pub mod lcr;
pub mod trunk_auth;
pub mod trunk_calls;
pub mod trunk_follow;
pub mod trunk_health;
pub mod trunk_quality;
pub mod trunk_refresh;
pub mod trunk_registration;
pub mod trunk_report;
pub mod trunk_router;
pub mod trunk_set;

// Re-export config types for convenience
pub use admission::{
    AdmissionAction, AdmissionConfig, AdmissionController, AdmissionCounters, AdmissionSection,
    AdmitResult, TenantCallLimits, TrunkCpsLimit, DEFAULT_REGISTER_CPS_PER_SOURCE,
};
pub use config::{SbcConfig, SbcStorageConfig};
pub use hmr::{
    HeaderManipulationConfig, HmrAction, HmrActionType, HmrCondition, HmrDirection, HmrMessageType,
    HmrRule,
};
pub use metrics::{PerSourceIpSnapshot, SbcMetricsConfig, SbcMetricsSnapshot, TrunkHealthStatus};
pub use nat::{NatConfig, StunServerConfig, TurnServerConfig};
pub use rate_limit::{MethodRateLimits, RateLimitEntry, RateLimitPolicyConfig};
pub use security::{AclAction, IpAcl, IpAclEntry, MalformedSipPolicy, SecurityConfig};
pub use siprec::{RecordingPolicy, RecordingServer, SelectiveRecordingRule, SiprecConfig};
pub use stir_shaken::{
    AttestationLevel, StirShakenConfig, StirSignMode, StirVerifyMode, TrunkStirShakenConfig,
};
pub use trunk::{
    SrtpMode, TrunkCodecConfig, TrunkConfig, TrunkCredentials, TrunkDirection, TrunkDtmfConfig,
    TrunkDtmfMode, TrunkFailover, TrunkSrtpConfig, TrunkTransport,
};

// Re-export LCR and quality types
pub use lcr::{LcrConfig, LcrRouteEntry, LoadBalanceStrategy};
pub use trunk_quality::{TrunkQualitySnapshot, TrunkQualityTracker};

// Re-export trunk runtime types for convenience
pub use cps::{CpsChecker, NoCpsLimit};
pub use drain::ClusterAnnouncer;
pub use fcp_core::trunk_report::{TrunkNodeReport, TrunkNodeView};
pub use health_probes::{run_health_probes, HealthAnnouncer};
pub use hmr_engine::HeaderManipulationEngine;
pub use trunk_auth::TrunkCredentialProvider;
pub use trunk_calls::{TrunkCalls, TrunkCps};
pub use trunk_follow::{follow_trunk_health, watch_trunk_health, TrunkViews};
pub use trunk_health::TrunkHealthMonitor;
pub use trunk_refresh::{
    refresh_routing_rules, refresh_trunks, spawn_routing_rule_refresh, spawn_trunk_refresh,
    LCR_ROUTE_ENTITY, ROUTING_RULE_ENTITY, TRUNK_ENTITY,
};
pub use trunk_registration::{
    run_trunk_registration, AloneOnThisNode, ClusterOwnership, RegistrationState, TrunkOwnership,
    TrunkRegistrar, TrunkRegistrationContext,
};
pub use trunk_report::{node_view, run_trunk_reporting};
pub use trunk_router::{PriceSource, RoutePrice, TrunkRouter};
pub use trunk_set::TrunkSet;
