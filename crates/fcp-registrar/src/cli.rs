//! Command-line interface for fcp-registrar.

use clap::Parser;
use std::path::PathBuf;

/// FCP Registrar - Standalone SIP Registration Server
#[derive(Parser, Debug)]
#[command(
    name = "fcp-registrar",
    version,
    about = "Standalone SIP registrar for the Ferrous Communications Platform",
    long_about = "A standalone SIP REGISTER server that handles device registration.\n\
                  Supports SQLite, PostgreSQL, MySQL, and MongoDB storage backends."
)]
pub struct Cli {
    /// Path to configuration file
    #[arg(
        short = 'c',
        long = "config",
        value_name = "FILE",
        help = "Path to TOML configuration file",
        env = "FCP_REGISTRAR_CONFIG"
    )]
    pub config: Option<PathBuf>,

    /// Log level override
    #[arg(
        short = 'l',
        long = "log-level",
        value_name = "LEVEL",
        help = "Override log level (trace, debug, info, warn, error)",
        env = "FCP_REGISTRAR_LOG_LEVEL"
    )]
    pub log_level: Option<String>,

    /// Log format
    #[arg(
        long = "log-format",
        value_name = "FORMAT",
        help = "Log format (json, pretty, compact)",
        env = "FCP_REGISTRAR_LOG_FORMAT"
    )]
    pub log_format: Option<String>,

    /// SIP bind address override
    #[arg(
        short = 'b',
        long = "sip-bind",
        value_name = "ADDR",
        help = "Override SIP UDP/TCP bind address (e.g., 0.0.0.0:5060)",
        env = "FCP_REGISTRAR_SIP_BIND"
    )]
    pub sip_bind: Option<String>,

    /// HTTP health endpoint bind address override
    #[arg(
        long = "http-bind",
        value_name = "ADDR",
        help = "Override HTTP health endpoint bind address (e.g., 0.0.0.0:8083)",
        env = "FCP_REGISTRAR_HTTP_BIND"
    )]
    pub http_bind: Option<String>,
}
