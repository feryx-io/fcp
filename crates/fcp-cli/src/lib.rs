//! FCP CLI library - Shared code for the CLI and TUI

pub mod api;
pub mod config;
pub mod output;

// Re-export commonly used types
pub use api::ApiClient;
pub use config::{AppSettings, Config, Context, ResolvedContext};
