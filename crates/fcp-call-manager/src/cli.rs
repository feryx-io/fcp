//! Command-line interface for fcp-call-manager.

use clap::Parser;
use std::path::PathBuf;

/// FCP Call Manager - Combined SIP Registrar and B2BUA Call Handler
#[derive(Parser, Debug)]
#[command(
    name = "fcp-call-manager",
    version,
    about = "Combined SIP registrar and B2BUA for the Ferrous Communications Platform",
    long_about = "A combined SIP registrar and B2BUA call handler.\n\
                  Users register their devices and place/receive calls through the same binary.\n\
                  Supports SQLite, PostgreSQL, MySQL, and MongoDB storage backends."
)]
pub struct Cli {
    /// Path to configuration file
    #[arg(
        short = 'c',
        long = "config",
        value_name = "FILE",
        help = "Path to TOML configuration file",
        env = "FCP_CALL_MANAGER_CONFIG"
    )]
    pub config: Option<PathBuf>,

    /// Log level override
    #[arg(
        short = 'l',
        long = "log-level",
        value_name = "LEVEL",
        help = "Override log level (trace, debug, info, warn, error)",
        env = "FCP_CALL_MANAGER_LOG_LEVEL"
    )]
    pub log_level: Option<String>,

    /// Log format
    #[arg(
        long = "log-format",
        value_name = "FORMAT",
        help = "Log format (json, pretty, compact)",
        env = "FCP_CALL_MANAGER_LOG_FORMAT"
    )]
    pub log_format: Option<String>,

    /// SIP bind address override
    #[arg(
        short = 'b',
        long = "sip-bind",
        value_name = "ADDR",
        help = "Override SIP UDP/TCP bind address (e.g., 0.0.0.0:5060)",
        env = "FCP_CALL_MANAGER_SIP_BIND"
    )]
    pub sip_bind: Option<String>,

    /// HTTP health endpoint bind address override
    #[arg(
        long = "http-bind",
        value_name = "ADDR",
        help = "Override HTTP health endpoint bind address (e.g., 0.0.0.0:8084)",
        env = "FCP_CALL_MANAGER_HTTP_BIND"
    )]
    pub http_bind: Option<String>,
}
