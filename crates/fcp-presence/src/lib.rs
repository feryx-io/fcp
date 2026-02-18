//! SIP Presence service for user availability tracking.
//!
//! This crate provides the core types, configuration, and runtime state for
//! a SIP presence service (RFC 3856/3903). It tracks user availability
//! (online, away, busy, on-the-phone, DND, offline) and manages watcher
//! subscriptions for SUBSCRIBE/NOTIFY.
//!
//! # Architecture
//!
//! The presence system is a **library crate** that provides:
//! - [`PresenceManager`]: Runtime state for presentities and watcher subscriptions
//! - [`PresenceConfig`]: Configuration for the presence service
//! - [`PresenceInfo`]: Current presence state for a user (presentity)
//! - PIDF XML generation/parsing for NOTIFY/PUBLISH bodies
//!
//! SIP signaling integration (SUBSCRIBE/NOTIFY/PUBLISH handlers) lives in
//! the binary crate (`fcp-call-manager` or `fcp-pbx`).

pub mod config;
pub mod dialog_info;
pub mod events;
pub mod manager;
pub mod pidf;
pub mod types;

pub use config::PresenceConfig;
pub use manager::PresenceManager;
pub use types::{PresenceInfo, PresenceState, PresenceUpdate, Watcher};
