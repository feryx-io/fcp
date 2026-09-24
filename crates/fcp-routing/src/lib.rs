//! FCP Routing - Declarative policy-based call routing.
//!
//! This crate provides a flexible, declarative routing system with five layers:
//! 1. **Identity Layer** - WHO is calling (users, roles, groups)
//! 2. **Reachability Layer** - WHAT is the destination (pattern matching, classification) [FUTURE]
//! 3. **Policy Layer** - IS IT ALLOWED (permit/deny rules)
//! 4. **Routing Layer** - HOW to route (trunk selection, failover) [FUTURE]
//! 5. **Transform Layer** - MODIFY (header/number manipulation) [FUTURE]
//!
//! The identity and policy layers live in the dedicated [`fcp_policy`] crate;
//! this crate re-exports them unchanged for backward compatibility and adds
//! the routing-facing [`RoutingService`] wrapper. New code should depend on
//! `fcp-policy` directly for policy concerns.
//!
//! ## Example
//!
//! ```rust,no_run
//! use fcp_routing::{RoutingService, YamlRoutingStore};
//! use fcp_core::TenantId;
//! use std::sync::Arc;
//! use std::path::PathBuf;
//!
//! # async fn example() -> anyhow::Result<()> {
//! // Create YAML-backed storage
//! let store = YamlRoutingStore::new(PathBuf::from("routing.yaml"), true).await?;
//!
//! // Create routing service
//! let service = RoutingService::new(Arc::new(store));
//!
//! // Evaluate a call
//! let tenant_id = TenantId::nil(); // or parse from config
//! let decision = service.evaluate_call(
//!     "alice@company.com",
//!     "sip:bob@company.com",
//!     &tenant_id,
//!     "10.0.1.100".parse()?,
//! ).await?;
//!
//! match decision.action {
//!     fcp_routing::PolicyAction::Allow => println!("Call allowed: {}", decision.reason),
//!     fcp_routing::PolicyAction::Deny => println!("Call denied: {}", decision.reason),
//! }
//! # Ok(())
//! # }
//! ```

pub mod service;
pub mod users;

// The policy engine was extracted into fcp-policy; re-export it wholesale so
// existing `fcp_routing::` paths keep working.
pub use fcp_policy::{evaluator, models, storage};

pub use fcp_policy::{
    AppliesTo, DestinationMatch, DeviceType, Endpoint, IdentityContext, IpRange, PolicyAction,
    PolicyDecision, PolicyEvaluator, PolicyRule, RoutingStore, TenantFeatures, TimeWindow, User,
    YamlRoutingStore,
};
pub use service::RoutingService;
pub use users::DatabaseRoutingStore;
