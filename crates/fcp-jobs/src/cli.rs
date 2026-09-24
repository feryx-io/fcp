//! Command-line interface for fcp-jobs.

use clap::Parser;
use std::path::PathBuf;

/// FCP Jobs - Async job processing service
#[derive(Parser, Debug)]
#[command(
    name = "fcp-jobs",
    version,
    about = "FCP Jobs - Async job processing service",
    long_about = "FCP Jobs processes background tasks asynchronously:\n\
                  - CDR (Call Detail Record) processing\n\
                  - Recording file processing and upload\n\
                  - Webhook delivery with retry logic\n\
                  - Scheduled cleanup tasks\n\
                  - Long-running operations\n\n\
                  This is a stateless worker service that can be scaled horizontally.\n\
                  Workers pull jobs from a Redis queue and process them concurrently."
)]
pub struct Cli {
    /// Path to configuration file
    #[arg(
        short = 'c',
        long = "config",
        value_name = "FILE",
        help = "Path to TOML configuration file",
        env = "FCP_JOBS_CONFIG"
    )]
    pub config: Option<PathBuf>,

    /// Log level override
    #[arg(
        short = 'l',
        long = "log-level",
        value_name = "LEVEL",
        help = "Override log level (trace, debug, info, warn, error)",
        env = "FCP_JOBS_LOG_LEVEL"
    )]
    pub log_level: Option<String>,

    /// Log format
    #[arg(
        long = "log-format",
        value_name = "FORMAT",
        help = "Log format (json, pretty, compact)",
        env = "FCP_JOBS_LOG_FORMAT"
    )]
    pub log_format: Option<String>,

    /// Number of worker threads
    #[arg(
        short = 'w',
        long = "workers",
        value_name = "COUNT",
        help = "Override number of worker threads",
        env = "FCP_JOBS_WORKERS"
    )]
    pub workers: Option<usize>,

    /// Redis URL override
    #[arg(
        long = "redis-url",
        value_name = "URL",
        help = "Override Redis connection URL",
        env = "FCP_JOBS_REDIS_URL"
    )]
    pub redis_url: Option<String>,
    /// Load and validate the configuration, print it with every secret
    /// masked, and exit: 0 when the node would start, 1 when a startup
    /// rule refuses it, 2 when it does not parse.
    #[arg(long = "check-config")]
    pub check_config: bool,
    /// `[cdr] database_url` and `[cdr_archive] database_url` from the environment, for a secret store or a compose
    /// `.env` to inject; wins over the file (`docs/SHIP_AND_OPERATE.md`
    /// phase 2).
    #[arg(
        long = "database-url",
        value_name = "URL",
        env = "FCP_JOBS_DATABASE_URL",
        hide_env_values = true
    )]
    pub database_url: Option<String>,
}
