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

    /// Public IP address for SIP Contact/Via headers and SDP media
    #[arg(
        long = "public-ip",
        value_name = "IP",
        help = "Public IP for SIP and RTP (required in Docker or behind NAT)",
        env = "FCP_CALL_MANAGER_PUBLIC_IP"
    )]
    pub public_ip: Option<String>,

    /// SIPS/TLS bind address (default: 0.0.0.0:5061)
    #[arg(
        long = "sips-bind",
        value_name = "ADDR",
        help = "SIPS (TLS) bind address (e.g., 0.0.0.0:5061)",
        env = "FCP_CALL_MANAGER_SIPS_BIND"
    )]
    pub sips_bind: Option<String>,

    /// SIP-over-WebSocket bind address (e.g. 0.0.0.0:5080)
    #[arg(
        long = "ws-bind",
        value_name = "ADDR",
        help = "SIP-over-WebSocket bind address (e.g., 0.0.0.0:5080)",
        env = "FCP_CALL_MANAGER_WS_BIND"
    )]
    pub ws_bind: Option<String>,

    /// Secure SIP-over-WebSocket bind address (e.g. 0.0.0.0:5081)
    #[arg(
        long = "wss-bind",
        value_name = "ADDR",
        help = "Secure SIP-over-WebSocket bind address (e.g., 0.0.0.0:5081)",
        env = "FCP_CALL_MANAGER_WSS_BIND"
    )]
    pub wss_bind: Option<String>,

    /// Path to TLS certificate file (PEM format)
    #[arg(
        long = "tls-cert",
        value_name = "FILE",
        help = "Path to TLS certificate (PEM format)",
        env = "FCP_CALL_MANAGER_TLS_CERT"
    )]
    pub tls_cert: Option<PathBuf>,

    /// Path to TLS private key file (PEM format)
    #[arg(
        long = "tls-key",
        value_name = "FILE",
        help = "Path to TLS private key (PEM format)",
        env = "FCP_CALL_MANAGER_TLS_KEY"
    )]
    pub tls_key: Option<PathBuf>,
    /// Load and validate the configuration, print it with every secret
    /// masked, and exit: 0 when the node would start, 1 when a startup
    /// rule refuses it, 2 when it does not parse.
    #[arg(long = "check-config")]
    pub check_config: bool,
}
