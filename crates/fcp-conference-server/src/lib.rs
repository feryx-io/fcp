//! fcp-conference-server library interface.
//!
//! Exposes config types for integration testing and external consumption.

pub mod config;
/// Recording a room to a file, and the shape of that decision. In the
/// library so `config`'s validation can reach it, as `config` itself is.
pub mod recording;

pub use config::{load_config, validate_config, ConferenceServerConfig};
