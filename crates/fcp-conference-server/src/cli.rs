use clap::Parser;
use std::path::PathBuf;

/// FCP Conference Server - Multi-Party Audio Conference Bridge
#[derive(Parser, Debug)]
#[command(
    name = "fcp-conference",
    version,
    about = "Standalone conference bridge server for the Ferrous Communications Platform",
    long_about = "A conference bridge that accepts SIP INVITEs, authenticates participants via PIN,\n\
                  mixes audio using forge-conference, and provides a REST API for meeting management.\n\
                  Supports SQLite, PostgreSQL, MySQL, and MongoDB storage backends."
)]
pub struct Cli {
    /// Path to configuration file
    #[arg(
        short = 'c',
        long = "config",
        value_name = "FILE",
        help = "Path to TOML configuration file",
        env = "FCP_CONFERENCE_CONFIG"
    )]
    pub config: Option<PathBuf>,

    /// Log level override
    #[arg(
        short = 'l',
        long = "log-level",
        value_name = "LEVEL",
        help = "Override log level (trace, debug, info, warn, error)",
        env = "FCP_CONFERENCE_LOG_LEVEL"
    )]
    pub log_level: Option<String>,

    /// Log format
    #[arg(
        long = "log-format",
        value_name = "FORMAT",
        help = "Log format (json, pretty, compact)",
        env = "FCP_CONFERENCE_LOG_FORMAT"
    )]
    pub log_format: Option<String>,

    /// SIP bind address override
    #[arg(
        short = 'b',
        long = "sip-bind",
        value_name = "ADDR",
        help = "Override SIP UDP/TCP bind address (e.g., 0.0.0.0:5060)",
        env = "FCP_CONFERENCE_SIP_BIND"
    )]
    pub sip_bind: Option<String>,

    /// HTTP health endpoint bind address override
    #[arg(
        long = "http-bind",
        value_name = "ADDR",
        help = "Override HTTP health endpoint bind address (e.g., 0.0.0.0:8090)",
        env = "FCP_CONFERENCE_HTTP_BIND"
    )]
    pub http_bind: Option<String>,

    /// Public IP address for SIP Contact/Via headers and SDP media
    #[arg(
        long = "public-ip",
        value_name = "IP",
        help = "Public IP for SIP and RTP (required in Docker or behind NAT)",
        env = "FCP_CONFERENCE_PUBLIC_IP"
    )]
    pub public_ip: Option<String>,
    /// Load and validate the configuration, print it with every secret
    /// masked, and exit: 0 when the node would start, 1 when a startup
    /// rule refuses it, 2 when it does not parse.
    #[arg(long = "check-config")]
    pub check_config: bool,
    /// `[storage] database_url` from the environment, for a secret store or a compose
    /// `.env` to inject; wins over the file (`docs/SHIP_AND_OPERATE.md`
    /// phase 2).
    #[arg(
        long = "database-url",
        value_name = "URL",
        env = "FCP_CONFERENCE_DATABASE_URL",
        hide_env_values = true
    )]
    pub database_url: Option<String>,

    /// `[http] auth_token` from the environment, for a secret store or a compose
    /// `.env` to inject; wins over the file (`docs/SHIP_AND_OPERATE.md`
    /// phase 2).
    #[arg(
        long = "auth-token",
        value_name = "TOKEN",
        env = "FCP_CONFERENCE_AUTH_TOKEN",
        hide_env_values = true
    )]
    pub auth_token: Option<String>,

    /// `[events] auth_token` from the environment, for a secret store or a compose
    /// `.env` to inject; wins over the file (`docs/SHIP_AND_OPERATE.md`
    /// phase 2).
    #[arg(
        long = "events-auth-token",
        value_name = "TOKEN",
        env = "FCP_CONFERENCE_EVENTS_AUTH_TOKEN",
        hide_env_values = true
    )]
    pub events_auth_token: Option<String>,

    /// `[conference] join_secret` from the environment, for a secret store or a compose
    /// `.env` to inject; wins over the file (`docs/SHIP_AND_OPERATE.md`
    /// phase 2).
    #[arg(
        long = "join-secret",
        value_name = "TOKEN",
        env = "FCP_CONFERENCE_JOIN_SECRET",
        hide_env_values = true
    )]
    pub join_secret: Option<String>,

    /// `[cluster] secret` from the environment, for a secret store or a compose
    /// `.env` to inject; wins over the file (`docs/SHIP_AND_OPERATE.md`
    /// phase 2).
    #[arg(
        long = "cluster-secret",
        value_name = "TOKEN",
        env = "FCP_CONFERENCE_CLUSTER_SECRET",
        hide_env_values = true
    )]
    pub cluster_secret: Option<String>,
}
