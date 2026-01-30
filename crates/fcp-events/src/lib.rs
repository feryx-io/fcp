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

#[cfg(test)]
mod tests;
