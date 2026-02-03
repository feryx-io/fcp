//! Command-line interface for fcp-voicemail-server.

use clap::Parser;
use std::path::PathBuf;

/// FCP Voicemail Server - Standalone Voicemail
#[derive(Parser, Debug)]
#[command(
    name = "fcp-voicemail",
    version,
    about = "Standalone voicemail server for the Ferrous Communications Platform",
    long_about = "A voicemail server that accepts SIP INVITEs for configured DIDs/extensions,\n\
                  plays greetings, records messages, and manages MWI notifications.\n\
                  Supports SQLite, PostgreSQL, MySQL, and MongoDB storage backends."
)]
pub struct Cli {
    /// Path to configuration file
    #[arg(
        short = 'c',
        long = "config",
        value_name = "FILE",
        help = "Path to TOML configuration file",
        env = "FCP_VOICEMAIL_CONFIG"
    )]
    pub config: Option<PathBuf>,

    /// Log level override
    #[arg(
        short = 'l',
        long = "log-level",
        value_name = "LEVEL",
        help = "Override log level (trace, debug, info, warn, error)",
        env = "FCP_VOICEMAIL_LOG_LEVEL"
    )]
    pub log_level: Option<String>,

    /// Log format
    #[arg(
        long = "log-format",
        value_name = "FORMAT",
        help = "Log format (json, pretty, compact)",
        env = "FCP_VOICEMAIL_LOG_FORMAT"
    )]
    pub log_format: Option<String>,

    /// SIP bind address override
    #[arg(
        short = 'b',
        long = "sip-bind",
        value_name = "ADDR",
        help = "Override SIP UDP/TCP bind address (e.g., 0.0.0.0:5060)",
        env = "FCP_VOICEMAIL_SIP_BIND"
    )]
    pub sip_bind: Option<String>,

    /// HTTP health endpoint bind address override
    #[arg(
        long = "http-bind",
        value_name = "ADDR",
        help = "Override HTTP health endpoint bind address (e.g., 0.0.0.0:8087)",
        env = "FCP_VOICEMAIL_HTTP_BIND"
    )]
    pub http_bind: Option<String>,

    /// Public IP address for SIP Contact/Via headers and SDP media
    #[arg(
        long = "public-ip",
        value_name = "IP",
        help = "Public IP for SIP and RTP (required in Docker or behind NAT)",
        env = "FCP_VOICEMAIL_PUBLIC_IP"
    )]
    pub public_ip: Option<String>,
}
