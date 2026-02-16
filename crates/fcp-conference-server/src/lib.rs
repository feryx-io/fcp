//! fcp-conference-server library interface.
//!
//! Exposes config types for integration testing and external consumption.

pub mod config;

pub use config::{load_config, validate_config, ConferenceServerConfig};
