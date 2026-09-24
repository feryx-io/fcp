//! Command-line interface for fcp-api-gateway.

use clap::Parser;
use std::path::PathBuf;

/// FCP API Gateway - Public-facing API with caching and rate limiting
#[derive(Parser, Debug)]
#[command(
    name = "fcp-api-gateway",
    version,
    about = "FCP API Gateway - Public-facing API with caching and rate limiting",
    long_about = "FCP API Gateway provides:\n\
                  - Request routing to backend services (admin, events, jobs)\n\
                  - Response caching (Redis-backed)\n\
                  - Rate limiting (per-tenant, per-endpoint)\n\
                  - JWT/Bearer token validation\n\
                  - CORS handling\n\
                  - Request aggregation\n\
                  - Health checks\n\n\
                  This is the public-facing API service that protects backend\n\
                  services from abuse and reduces database load through caching."
)]
pub struct Cli {
    /// Path to configuration file
    #[arg(
        short = 'c',
        long = "config",
        value_name = "FILE",
        help = "Path to TOML configuration file",
        env = "FCP_GATEWAY_CONFIG"
    )]
    pub config: Option<PathBuf>,

    /// Log level override
    #[arg(
        short = 'l',
        long = "log-level",
        value_name = "LEVEL",
        help = "Override log level (trace, debug, info, warn, error)",
        env = "FCP_GATEWAY_LOG_LEVEL"
    )]
    pub log_level: Option<String>,

    /// Log format
    #[arg(
        long = "log-format",
        value_name = "FORMAT",
        help = "Log format (json, pretty, compact)",
        env = "FCP_GATEWAY_LOG_FORMAT"
    )]
    pub log_format: Option<String>,

    /// Bind address for gateway
    #[arg(
        short = 'b',
        long = "bind",
        value_name = "ADDR",
        help = "Override gateway bind address (e.g., 0.0.0.0:443)",
        env = "FCP_GATEWAY_BIND_ADDR"
    )]
    pub bind_addr: Option<String>,

    /// Redis URL override
    #[arg(
        long = "redis-url",
        value_name = "URL",
        help = "Override Redis connection URL",
        env = "FCP_GATEWAY_REDIS_URL"
    )]
    pub redis_url: Option<String>,
    /// Load and validate the configuration, print it with every secret
    /// masked, and exit: 0 when the node would start, 1 when a startup
    /// rule refuses it, 2 when it does not parse.
    #[arg(long = "check-config")]
    pub check_config: bool,
}
