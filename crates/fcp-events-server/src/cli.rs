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
        help = "Override WebSocket bind address (e.g., 127.0.0.1:8081 or 0.0.0.0:8081)",
        env = "FCP_EVENTS_BIND_ADDR"
    )]
    pub bind_addr: Option<String>,
    /// Load and validate the configuration, print it with every secret
    /// masked, and exit: 0 when the node would start, 1 when a startup
    /// rule refuses it, 2 when it does not parse.
    #[arg(long = "check-config")]
    pub check_config: bool,
    /// `[auth] token` from the environment, for a secret store or a compose
    /// `.env` to inject; wins over the file (`docs/SHIP_AND_OPERATE.md`
    /// phase 2).
    #[arg(
        long = "auth-token",
        value_name = "TOKEN",
        env = "FCP_EVENTS_AUTH_TOKEN",
        hide_env_values = true
    )]
    pub auth_token: Option<String>,

    /// `[auth] session_signing_key` from the environment: the admin's
    /// session key (`docs/IDENTITY_AND_DEVICES.md` phase 1).
    #[arg(
        long = "session-signing-key",
        value_name = "KEY",
        env = "FCP_EVENTS_SESSION_SIGNING_KEY",
        hide_env_values = true
    )]
    pub session_signing_key: Option<String>,

    /// `[webhook] database_url` from the environment, for a secret store or a compose
    /// `.env` to inject; wins over the file (`docs/SHIP_AND_OPERATE.md`
    /// phase 2).
    #[arg(
        long = "webhook-database-url",
        value_name = "URL",
        env = "FCP_EVENTS_WEBHOOK_DATABASE_URL",
        hide_env_values = true
    )]
    pub webhook_database_url: Option<String>,

    /// The call manager's API token for emergency notification calls
    /// (`[notifications.call] auth_token`); the call channel must already
    /// be configured.
    #[arg(
        long = "notify-call-token",
        value_name = "TOKEN",
        env = "FCP_EVENTS_NOTIFY_CALL_TOKEN",
        hide_env_values = true
    )]
    pub notify_call_token: Option<String>,
}
