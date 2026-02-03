//! Command-line interface for fcp-ivr.

use clap::Parser;
use std::path::PathBuf;

/// FCP IVR Server - Standalone Interactive Voice Response
#[derive(Parser, Debug)]
#[command(
    name = "fcp-ivr",
    version,
    about = "Standalone IVR server for the Ferrous Communications Platform",
    long_about = "An IVR server that accepts SIP INVITEs, matches DIDs to programmed flows,\n\
                  and executes them. Supports Play, Say, Gather, Dial, and other flow nodes.\n\
                  Supports SQLite, PostgreSQL, MySQL, and MongoDB storage backends."
)]
pub struct Cli {
    /// Path to configuration file
    #[arg(
        short = 'c',
        long = "config",
        value_name = "FILE",
        help = "Path to TOML configuration file",
        env = "FCP_IVR_CONFIG"
    )]
    pub config: Option<PathBuf>,

    /// Log level override
    #[arg(
        short = 'l',
        long = "log-level",
        value_name = "LEVEL",
        help = "Override log level (trace, debug, info, warn, error)",
        env = "FCP_IVR_LOG_LEVEL"
    )]
    pub log_level: Option<String>,

    /// Log format
    #[arg(
        long = "log-format",
        value_name = "FORMAT",
        help = "Log format (json, pretty, compact)",
        env = "FCP_IVR_LOG_FORMAT"
    )]
    pub log_format: Option<String>,

    /// SIP bind address override
    #[arg(
        short = 'b',
        long = "sip-bind",
        value_name = "ADDR",
        help = "Override SIP UDP/TCP bind address (e.g., 0.0.0.0:5060)",
        env = "FCP_IVR_SIP_BIND"
    )]
    pub sip_bind: Option<String>,

    /// HTTP health endpoint bind address override
    #[arg(
        long = "http-bind",
        value_name = "ADDR",
        help = "Override HTTP health endpoint bind address (e.g., 0.0.0.0:8086)",
        env = "FCP_IVR_HTTP_BIND"
    )]
    pub http_bind: Option<String>,

    /// Public IP address for SIP Contact/Via headers and SDP media
    #[arg(
        long = "public-ip",
        value_name = "IP",
        help = "Public IP for SIP and RTP (required in Docker or behind NAT)",
        env = "FCP_IVR_PUBLIC_IP"
    )]
    pub public_ip: Option<String>,
}
