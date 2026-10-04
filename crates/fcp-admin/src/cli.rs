//! Command-line interface for fcp-admin.

use clap::Parser;
use std::path::PathBuf;

/// FCP Admin - Management and Administration API
#[derive(Parser, Debug)]
#[command(
    name = "fcp-admin",
    version,
    about = "FCP Admin API - Management and administration endpoints",
    long_about = "FCP Admin API provides management and administration endpoints for:\n\
                  - Tenant management\n\
                  - User administration\n\
                  - Device provisioning\n\
                  - Extension configuration\n\
                  - Phone number management\n\
                  - System configuration\n\n\
                  This is an internal-only API service designed to run behind\n\
                  an API gateway. It focuses on CRUD operations with optimized\n\
                  database access."
)]
pub struct Cli {
    /// Path to configuration file
    #[arg(
        short = 'c',
        long = "config",
        value_name = "FILE",
        help = "Path to TOML configuration file",
        env = "FCP_ADMIN_CONFIG"
    )]
    pub config: Option<PathBuf>,

    /// Log level override
    #[arg(
        short = 'l',
        long = "log-level",
        value_name = "LEVEL",
        help = "Override log level (trace, debug, info, warn, error)",
        env = "FCP_ADMIN_LOG_LEVEL"
    )]
    pub log_level: Option<String>,

    /// Log format
    #[arg(
        long = "log-format",
        value_name = "FORMAT",
        help = "Log format (json, pretty, compact)",
        env = "FCP_ADMIN_LOG_FORMAT"
    )]
    pub log_format: Option<String>,

    /// Bind address for API
    #[arg(
        short = 'b',
        long = "bind",
        value_name = "ADDR",
        help = "Override API bind address (e.g., 0.0.0.0:8080)",
        env = "FCP_ADMIN_BIND_ADDR"
    )]
    pub bind_addr: Option<String>,

    /// Authentication token
    #[arg(
        long = "auth-token",
        value_name = "TOKEN",
        help = "Bearer authentication token",
        env = "FCP_ADMIN_AUTH_TOKEN"
    )]
    pub auth_token: Option<String>,

    /// The key sign-in sessions are signed with (`[api.session] signing_key`)
    #[arg(
        long = "session-signing-key",
        value_name = "KEY",
        help = "Key sign-in sessions are signed with (32 characters or more)",
        env = "FCP_ADMIN_SESSION_SIGNING_KEY"
    )]
    pub session_signing_key: Option<String>,

    /// Disable the HTTPS requirement for the provisioning endpoint
    #[arg(
        long = "provisioning-no-https",
        help = "Allow provisioning over plain HTTP (not recommended; credentials may be exposed)",
        env = "FCP_ADMIN_PROVISIONING_NO_HTTPS"
    )]
    pub provisioning_no_https: bool,
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
        env = "FCP_ADMIN_DATABASE_URL",
        hide_env_values = true
    )]
    pub database_url: Option<String>,

    /// `[events] auth_token` from the environment, for a secret store or a compose
    /// `.env` to inject; wins over the file (`docs/SHIP_AND_OPERATE.md`
    /// phase 2).
    #[arg(
        long = "events-auth-token",
        value_name = "TOKEN",
        env = "FCP_ADMIN_EVENTS_AUTH_TOKEN",
        hide_env_values = true
    )]
    pub events_auth_token: Option<String>,

    /// `[call_manager] auth_token` from the environment, for a secret store or a compose
    /// `.env` to inject; wins over the file (`docs/SHIP_AND_OPERATE.md`
    /// phase 2).
    #[arg(
        long = "call-manager-auth-token",
        value_name = "TOKEN",
        env = "FCP_ADMIN_CALL_MANAGER_AUTH_TOKEN",
        hide_env_values = true
    )]
    pub call_manager_auth_token: Option<String>,

    /// `[conference] auth_token` from the environment, for a secret store or a compose
    /// `.env` to inject; wins over the file (`docs/SHIP_AND_OPERATE.md`
    /// phase 2).
    #[arg(
        long = "conference-auth-token",
        value_name = "TOKEN",
        env = "FCP_ADMIN_CONFERENCE_AUTH_TOKEN",
        hide_env_values = true
    )]
    pub conference_auth_token: Option<String>,

    /// `[conference] join_secret` from the environment, for a secret store or a compose
    /// `.env` to inject; wins over the file (`docs/SHIP_AND_OPERATE.md`
    /// phase 2).
    #[arg(
        long = "conference-join-secret",
        value_name = "TOKEN",
        env = "FCP_ADMIN_CONFERENCE_JOIN_SECRET",
        hide_env_values = true
    )]
    pub conference_join_secret: Option<String>,
}
