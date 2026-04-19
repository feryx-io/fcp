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

    /// Device auto-provisioning token
    #[arg(
        long = "provisioning-auth-token",
        value_name = "TOKEN",
        help = "Shared token accepted only by the device provisioning endpoint",
        env = "FCP_ADMIN_PROVISIONING_AUTH_TOKEN"
    )]
    pub provisioning_auth_token: Option<String>,

    /// Disable the HTTPS requirement for the provisioning endpoint
    #[arg(
        long = "provisioning-no-https",
        help = "Allow provisioning over plain HTTP (not recommended; credentials may be exposed)",
        env = "FCP_ADMIN_PROVISIONING_NO_HTTPS"
    )]
    pub provisioning_no_https: bool,
}
