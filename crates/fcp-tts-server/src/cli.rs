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
}
