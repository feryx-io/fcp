//! FCP Policy - the dedicated policy engine.
//!
//! Extracted from `fcp-routing` (which re-exports everything here for
//! backward compatibility). The engine answers *is this call allowed?* for a
//! tenant, based on declarative rules:
//!
//! - **Identity** — who is calling (users, roles, groups)
//! - **Destination** — regex match on the dialed target
//! - **Time windows** — day-of-week + time-of-day, evaluated in the rule's
//!   own timezone (overnight windows spanning midnight are supported)
//! - **Source IP** — CIDR ranges
//! - **Tenant feature toggles** — rules can require a named feature that a
//!   tenant has enabled (e.g. `international_calls`)
//!
//! Rules are sorted by priority; the first match wins; the default is deny.
//! The reference store is YAML-backed with hot reload
//! ([`YamlPolicyStore`]); production deployments can implement
//! [`PolicyStore`] over any backend.

pub mod evaluator;
pub mod models;
pub mod storage;

// Canonical names
pub use evaluator::PolicyEvaluator;
pub use models::identity::{DeviceType, Endpoint, IdentityContext, User};
pub use models::policy::{
    AppliesTo, DestinationMatch, IpRange, PolicyAction, PolicyDecision, PolicyRule, TenantFeatures,
    TimeWindow,
};
pub use storage::yaml::YamlRoutingStore;
pub use storage::RoutingStore;

// Preferred aliases under the crate's own vocabulary. `RoutingStore` /
// `YamlRoutingStore` keep their historical names for the fcp-routing
// re-exports; these are the same items.
pub use storage::yaml::YamlRoutingStore as YamlPolicyStore;
pub use storage::RoutingStore as PolicyStore;
