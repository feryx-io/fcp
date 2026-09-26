//! Event system for FCP.
//!
//! This crate provides an event bus architecture for publishing and subscribing
//! to platform events such as call lifecycle, media events, recordings, etc.

pub mod bus;
pub mod filter;
pub mod types;

// Re-exports
pub use bus::{EventBus, EventPublisher, EventStream, EventSubscriber, InMemoryEventBus};
pub use filter::EventFilter;
pub use types::*;

/// The header a service posting to the events server's `/ingest` names
/// itself with.
pub const NODE_ID_HEADER: &str = "x-node-id";
/// The header naming the site whose events are being posted up its link
/// (`docs/HYBRID_CONTROL_PLANE.md` §3.3): the envelope carries it as
/// `origin_site`, and the events fire webhooks here, once.
pub const ORIGIN_SITE_HEADER: &str = "x-origin-site";

#[cfg(test)]
mod tests;
