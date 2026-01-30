//! Command-line interface for fcp-sbc.

use clap::Parser;
use std::path::PathBuf;

/// FCP SBC - Session Border Controller
#[derive(Parser, Debug)]
#[command(
    name = "fcp-sbc",
    version,
    about = "Session Border Controller for the Ferrous Communications Platform",
    long_about = "An SBC that enforces IP ACLs, message size limits, rate limiting,\n\
                  and admission control before delegating to the B2BUA handler.\n\
                  Supports trunk routing and health monitoring."
)]
pub struct Cli {
    /// SIP bind address (UDP/TCP)
    #[arg(
        short = 'b',
        long = "bind",
        value_name = "ADDR",
        default_value = "0.0.0.0:5060",
        help = "SIP UDP/TCP bind address",
        env = "FCP_SBC_BIND"
    )]
    pub bind: String,

    /// Path to configuration file
    #[arg(
        short = 'c',
        long = "config",
        value_name = "FILE",
        help = "Path to TOML configuration file",
        env = "FCP_SBC_CONFIG"
    )]
    pub config: Option<PathBuf>,

    /// Log level
    #[arg(
        short = 'l',
        long = "log-level",
        value_name = "LEVEL",
        default_value = "info",
        help = "Log level (trace, debug, info, warn, error)",
        env = "FCP_SBC_LOG_LEVEL"
    )]
    pub log_level: String,

    /// Log format
    #[arg(
        long = "log-format",
        value_name = "FORMAT",
        default_value = "json",
        help = "Log format (json, pretty, compact)",
        env = "FCP_SBC_LOG_FORMAT"
    )]
    pub log_format: String,

    /// HTTP health endpoint bind address
    #[arg(
        long = "health-bind",
        value_name = "ADDR",
        default_value = "127.0.0.1:8080",
        help = "HTTP health/readiness/metrics bind address",
        env = "FCP_SBC_HEALTH_BIND"
    )]
    pub health_bind: String,
}
