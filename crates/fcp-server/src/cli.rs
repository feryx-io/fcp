//! Command-line interface for fcp-server.

use clap::Parser;
use std::path::PathBuf;

/// FCP Server - Ferrous Communications Platform
#[derive(Parser, Debug)]
#[command(
    name = "fcp-server",
    version,
    about = "Ferrous Communications Platform server",
    long_about = "A high-performance, RFC-compliant SIP server built in Rust.\n\
                  Supports voice calls, messaging, presence, and more."
)]
pub struct Cli {
    /// Path to configuration file
    #[arg(
        short = 'c',
        long = "config",
        value_name = "FILE",
        help = "Path to TOML configuration file",
        env = "FCP_CONFIG"
    )]
    pub config: Option<PathBuf>,

    /// Log level override
    #[arg(
        short = 'l',
        long = "log-level",
        value_name = "LEVEL",
        help = "Override log level (trace, debug, info, warn, error)",
        env = "FCP_LOG_LEVEL"
    )]
    pub log_level: Option<String>,

    /// Log format
    #[arg(
        long = "log-format",
        value_name = "FORMAT",
        help = "Log format (json, pretty, compact)",
        env = "FCP_LOG_FORMAT"
    )]
    pub log_format: Option<String>,

    /// Node name override
    #[arg(
        short = 'n',
        long = "node-name",
        value_name = "NAME",
        help = "Override node name from config",
        env = "FCP_NODE_NAME"
    )]
    pub node_name: Option<String>,

    /// Bind address for SIP
    #[arg(
        short = 'b',
        long = "bind",
        value_name = "ADDR",
        help = "Override SIP bind address (e.g., 0.0.0.0:5060)",
        env = "FCP_BIND_ADDR"
    )]
    pub bind_addr: Option<String>,
}
