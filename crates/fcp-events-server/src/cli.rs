//! Command-line interface for fcp-events-server.

use clap::Parser;
use std::path::PathBuf;

/// FCP Events Server - WebSocket event streaming
#[derive(Parser, Debug)]
#[command(
    name = "fcp-events-server",
    version,
    about = "FCP Events Server - WebSocket event streaming",
    long_about = "FCP Events Server provides real-time event streaming via WebSocket:\n\
                  - WebSocket connections for event delivery\n\
                  - Event filtering and subscriptions\n\
                  - Connection management and heartbeat\n\
                  - Event batching and backpressure\n\
                  - Tenant isolation\n\n\
                  This is a stateful service designed for sticky sessions.\n\
                  Clients connect via WebSocket and receive filtered events\n\
                  based on their subscriptions."
)]
pub struct Cli {
    /// Path to configuration file
    #[arg(
        short = 'c',
        long = "config",
        value_name = "FILE",
        help = "Path to TOML configuration file",
        env = "FCP_EVENTS_CONFIG"
    )]
    pub config: Option<PathBuf>,

    /// Log level override
    #[arg(
        short = 'l',
        long = "log-level",
        value_name = "LEVEL",
        help = "Override log level (trace, debug, info, warn, error)",
        env = "FCP_EVENTS_LOG_LEVEL"
    )]
    pub log_level: Option<String>,

    /// Log format
    #[arg(
        long = "log-format",
        value_name = "FORMAT",
        help = "Log format (json, pretty, compact)",
        env = "FCP_EVENTS_LOG_FORMAT"
    )]
    pub log_format: Option<String>,

    /// Bind address for WebSocket server
    #[arg(
        short = 'b',
        long = "bind",
        value_name = "ADDR",
        help = "Override WebSocket bind address (e.g., 0.0.0.0:8081)",
        env = "FCP_EVENTS_BIND_ADDR"
    )]
    pub bind_addr: Option<String>,
}
