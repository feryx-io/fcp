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

    /// SIP TLS/SIPS bind address override
    #[arg(
        long = "sips-bind",
        value_name = "ADDR",
        help = "Override SIP TLS bind address (e.g., 0.0.0.0:5061)",
        env = "FCP_REGISTRAR_SIPS_BIND"
    )]
    pub sips_bind: Option<String>,

    /// SIP-over-WebSocket bind address override
    #[arg(
        long = "ws-bind",
        value_name = "ADDR",
        help = "Override SIP-over-WebSocket bind address",
        env = "FCP_REGISTRAR_WS_BIND"
    )]
    pub ws_bind: Option<String>,

    /// Secure SIP-over-WebSocket bind address override
    #[arg(
        long = "wss-bind",
        value_name = "ADDR",
        help = "Override secure SIP-over-WebSocket bind address",
        env = "FCP_REGISTRAR_WSS_BIND"
    )]
    pub wss_bind: Option<String>,

    /// Path to TLS certificate file (PEM)
    #[arg(
        long = "tls-cert",
        value_name = "FILE",
        help = "Path to TLS certificate file for SIPS/WSS",
        env = "FCP_REGISTRAR_TLS_CERT"
    )]
    pub tls_cert: Option<PathBuf>,

    /// Path to TLS private key file (PEM)
    #[arg(
        long = "tls-key",
        value_name = "FILE",
        help = "Path to TLS private key file for SIPS/WSS",
        env = "FCP_REGISTRAR_TLS_KEY"
    )]
    pub tls_key: Option<PathBuf>,

    /// HTTP health endpoint bind address override
    #[arg(
        long = "http-bind",
        value_name = "ADDR",
        help = "Override HTTP health endpoint bind address (e.g., 0.0.0.0:8083)",
        env = "FCP_REGISTRAR_HTTP_BIND"
    )]
    pub http_bind: Option<String>,
}
