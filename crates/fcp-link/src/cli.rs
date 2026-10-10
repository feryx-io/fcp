use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "fcp-link",
    version,
    about = "FCP Link - a site's link to its cloud",
    long_about = "fcp-link runs at a site (a tenant's own stack on its own network) and\n\
                  keeps it linked to the cloud (docs/HYBRID_CONTROL_PLANE.md): it enrolls\n\
                  once with a one-time token and holds a certificate from then on, and\n\
                  heartbeats what it finds at the site, and, with the site's Postgres,\n\
                  keeps it a copy of the tenant's configuration from the cloud."
)]
pub struct Cli {
    #[arg(
        short = 'c',
        long = "config",
        value_name = "FILE",
        help = "Path to TOML configuration file",
        env = "FCP_LINK_CONFIG"
    )]
    pub config: Option<PathBuf>,

    #[arg(
        short = 'l',
        long = "log-level",
        value_name = "LEVEL",
        help = "Override log level (trace, debug, info, warn, error)",
        env = "FCP_LINK_LOG_LEVEL"
    )]
    pub log_level: Option<String>,

    #[arg(
        long = "log-format",
        value_name = "FORMAT",
        help = "Log format (json, pretty, compact)",
        env = "FCP_LINK_LOG_FORMAT"
    )]
    pub log_format: Option<String>,

    /// Print the configuration this process would run with, masked, and exit.
    #[arg(long = "check-config")]
    pub check_config: bool,

    /// The one-time enrollment token from the cloud's `POST /v1/sites`.
    #[arg(
        long = "enrollment-token",
        value_name = "TOKEN",
        env = "FCP_LINK_ENROLLMENT_TOKEN",
        hide_env_values = true
    )]
    pub enrollment_token: Option<String>,

    /// The cloud's enroll listener, e.g. https://cloud.example.com:8446
    #[arg(long = "cloud-url", value_name = "URL", env = "FCP_LINK_CLOUD_URL")]
    pub cloud_url: Option<String>,

    /// The cloud's link listener, e.g. https://cloud.example.com:8447
    #[arg(long = "link-url", value_name = "URL", env = "FCP_LINK_LINK_URL")]
    pub link_url: Option<String>,

    /// The site's Postgres, kept a copy of the tenant's configuration.
    #[arg(
        long = "site-database-url",
        value_name = "URL",
        env = "FCP_LINK_SITE_DATABASE_URL",
        hide_env_values = true
    )]
    pub site_database_url: Option<String>,

    /// The token the site's call managers send their pushes with
    /// (`[push.relay] token`); unset, the link relays none.
    #[arg(
        long = "push-token",
        value_name = "TOKEN",
        env = "FCP_LINK_PUSH_TOKEN",
        hide_env_values = true
    )]
    pub push_token: Option<String>,
}
