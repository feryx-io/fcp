//! Core primitives shared across the FCP project.
//!
//! This crate holds common error types, identifiers, and configuration
//! structures that other crates depend on.

pub mod codec;
pub mod config;
pub mod error;
pub mod identifiers;
pub mod service;
pub mod time;
pub mod transport;

pub use crate::error::{Error, Result};
pub use crate::identifiers::*;
