use clap::Parser;

/// FCP TTS Server — shared Text-to-Speech service.
#[derive(Parser, Debug)]
#[command(name = "fcp-tts", version = env!("CARGO_PKG_VERSION"))]
pub struct Cli {
    /// Path to TOML configuration file.
    #[arg(
        short = 'c',
        long = "config",
        value_name = "PATH",
        env = "FCP_TTS_CONFIG"
    )]
    pub config: Option<String>,

    /// Override log level (trace, debug, info, warn, error).
    #[arg(
        short = 'l',
        long = "log-level",
        value_name = "LEVEL",
        env = "FCP_TTS_LOG_LEVEL"
    )]
    pub log_level: Option<String>,

    /// Log format (pretty, json, compact).
    #[arg(long = "log-format", value_name = "FMT", env = "FCP_TTS_LOG_FORMAT")]
    pub log_format: Option<String>,

    /// HTTP API bind address override.
    #[arg(long = "bind", value_name = "ADDR", env = "FCP_TTS_BIND_ADDR")]
    pub bind: Option<String>,
    /// Load and validate the configuration, print it with every secret
    /// masked, and exit: 0 when the node would start, 1 when a startup
    /// rule refuses it, 2 when it does not parse.
    #[arg(long = "check-config")]
    pub check_config: bool,
    /// `[auth] tokens`, comma-separated from the environment, for a secret store or a compose
    /// `.env` to inject; wins over the file (`docs/SHIP_AND_OPERATE.md`
    /// phase 2).
    #[arg(
        long = "auth-tokens",
        value_name = "TOKEN",
        env = "FCP_TTS_AUTH_TOKENS",
        hide_env_values = true
    )]
    pub auth_tokens: Option<String>,
}
