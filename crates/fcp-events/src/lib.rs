//! Event system for FCP.
//!
//! This crate provides an event bus architecture for publishing and subscribing
//! to platform events such as call lifecycle, media events, recordings, etc.

pub mod types;
pub mod bus;
pub mod filter;

// Re-exports
pub use types::*;
pub use bus::{EventBus, EventPublisher, EventSubscriber, EventStream, InMemoryEventBus};
pub use filter::EventFilter;

#[cfg(test)]
mod tests;