//! FCP CLI - Command-line interface for the Ferrous Communications Platform
//!
//! This tool provides both an interactive TUI mode and traditional command-line interface
//! for managing FCP servers.

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};

mod api;
mod commands;
mod config;
mod output;
mod tui;

#[derive(Parser)]
#[command(name = "fcp")]
#[command(version, about = "FCP CLI - Manage your FCP server", long_about = None)]
struct Cli {
    /// FCP server URL (overrides context)
    #[arg(long, env = "FCP_URL", global = true)]
    url: Option<String>,

    /// Authentication token (overrides context)
    #[arg(long, env = "FCP_TOKEN", global = true)]
    token: Option<String>,

    /// Context name to use
    #[arg(long, env = "FCP_CONTEXT", global = true)]
    context: Option<String>,

    /// Output format
    #[arg(long, short = 'o', default_value = "table", global = true)]
    output: OutputFormat,

    /// Disable color output
    #[arg(long, global = true)]
    no_color: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Clone, ValueEnum)]
enum OutputFormat {
    Table,
    Json,
    Yaml,
}

#[derive(Subcommand)]
enum Commands {
    /// Launch interactive TUI mode
    Tui,

    /// User management commands
    #[command(subcommand)]
    User(UserCommands),

    /// Call management commands
    #[command(subcommand)]
    Call(CallCommands),

    /// Registration management commands
    #[command(subcommand)]
    Registration(RegistrationCommands),

    /// Context management commands
    #[command(subcommand)]
    Context(ContextCommands),

    /// System information commands
    #[command(subcommand)]
    System(SystemCommands),

    /// Debug commands
    #[command(subcommand)]
    Debug(DebugCommands),
}

#[derive(Debug, Subcommand)]
enum UserCommands {
    /// List all users
    List,

    /// Show detailed information for a user
    Show {
        /// Username to display
        username: String,
    },

    /// Add a new user
    Add {
        /// Username
        username: String,

        /// Password (will prompt if not provided)
        #[arg(short, long)]
        password: Option<String>,

        /// Display name
        #[arg(short, long)]
        display_name: Option<String>,

        /// Tenant ID (defaults to nil tenant)
        #[arg(long)]
        tenant_id: Option<String>,

        /// Enable the user (default: true)
        #[arg(long, default_value = "true")]
        enabled: bool,
    },

    /// Modify an existing user
    Modify {
        /// Username to modify
        username: String,

        /// New display name
        #[arg(long)]
        display_name: Option<String>,

        /// Enable or disable the user
        #[arg(long)]
        enabled: Option<bool>,
    },

    /// Change user password
    Password {
        /// Username
        username: String,

        /// New password (will prompt if not provided)
        #[arg(short, long)]
        new_password: Option<String>,
    },

    /// Delete a user
    Delete {
        /// Username to delete
        username: String,

        /// Skip confirmation prompt
        #[arg(short = 'y', long)]
        yes: bool,
    },
}

#[derive(Debug, Subcommand)]
enum CallCommands {
    /// List all active calls
    List,

    /// Show detailed information for a call
    Show {
        /// Call ID
        call_id: String,
    },

    /// Hang up a call
    Hangup {
        /// Call ID
        call_id: String,

        /// Skip confirmation prompt
        #[arg(short = 'y', long)]
        yes: bool,
    },

    /// Originate a new call (future feature)
    Originate {
        /// From URI
        from: String,

        /// To URI
        to: String,

        /// Display name for caller
        #[arg(long)]
        from_display_name: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
enum RegistrationCommands {
    /// List registrations for an Address of Record
    List {
        /// Address of Record (e.g., sip:alice@example.com or alice@example.com)
        aor: String,
    },

    /// Delete all registrations for an AOR
    Delete {
        /// Address of Record
        aor: String,

        /// Specific contact to delete (if not provided, deletes all)
        #[arg(short, long)]
        contact: Option<String>,

        /// Skip confirmation prompt
        #[arg(short = 'y', long)]
        yes: bool,
    },
}

#[derive(Subcommand)]
enum ContextCommands {
    /// List all configured contexts
    List,

    /// Show the current context
    Current,

    /// Add a new context
    Add {
        /// Context name
        name: String,

        /// Server URL
        url: String,

        /// Authentication token
        #[arg(long)]
        token: Option<String>,
    },

    /// Switch to a different context
    Use {
        /// Context name
        name: String,
    },

    /// Delete a context
    Delete {
        /// Context name
        name: String,

        /// Skip confirmation prompt
        #[arg(short = 'y', long)]
        yes: bool,
    },

    /// Show details for a context
    Show {
        /// Context name (defaults to current)
        name: Option<String>,
    },
}

#[derive(Subcommand)]
enum SystemCommands {
    /// Check server health
    Health,

    /// Show detailed server status
    Status,
}

#[derive(Debug, Subcommand)]
enum DebugCommands {
    /// Show debug summary
    Summary,

    /// List active SIP dialogs
    Dialogs,

    /// List active subscriptions
    Subscriptions,

    /// Show rate limit information
    RateLimits,

    /// List active calls (debug view)
    Calls,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Parse command-line arguments
    let cli = Cli::parse();

    // Initialize colored output
    if cli.no_color {
        colored::control::set_override(false);
    }

    // Initialize tracing if RUST_LOG is set
    if std::env::var("RUST_LOG").is_ok() {
        tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .init();
    }

    // Load configuration and resolve context
    let config = config::Config::load_or_create()?;
    let resolved_context = resolve_context(&cli, &config)?;

    // Execute command or launch TUI
    match cli.command {
        None => {
            // Default to TUI mode
            tui::run(resolved_context).await?;
        }
        Some(Commands::Tui) => {
            // Explicitly requested TUI mode
            tui::run(resolved_context).await?;
        }
        Some(command) => {
            execute_command(command, resolved_context, cli.output).await?;
        }
    }

    Ok(())
}

/// Resolve the context to use based on CLI arguments and config
fn resolve_context(cli: &Cli, config: &config::Config) -> Result<config::ResolvedContext> {
    // If URL is provided on CLI, use it directly
    if let Some(url) = &cli.url {
        return Ok(config::ResolvedContext {
            name: "cli-override".to_string(),
            url: url.clone(),
            token: cli.token.clone(),
        });
    }

    // If context name is specified, use that context
    if let Some(context_name) = &cli.context {
        let context = config
            .get_context(context_name)
            .ok_or_else(|| anyhow::anyhow!("Context '{}' not found", context_name))?;

        return Ok(config::ResolvedContext {
            name: context.name.clone(),
            url: context.url.clone(),
            token: cli.token.clone().or_else(|| context.token.clone()),
        });
    }

    // Use current context from config
    let context = config
        .current()
        .ok_or_else(|| anyhow::anyhow!("No current context configured. Use 'fcp context add' to add one."))?;

    Ok(config::ResolvedContext {
        name: context.name.clone(),
        url: context.url.clone(),
        token: cli.token.clone().or_else(|| context.token.clone()),
    })
}

/// Execute a command in non-TUI mode
async fn execute_command(
    command: Commands,
    context: config::ResolvedContext,
    output_format: OutputFormat,
) -> Result<()> {
    match command {
        Commands::Tui => unreachable!("TUI handled in main"),

        Commands::User(cmd) => {
            commands::user::handle(cmd, context, output_format).await?;
        }

        Commands::Call(cmd) => {
            commands::call::handle(cmd, context, output_format).await?;
        }

        Commands::Registration(cmd) => {
            commands::registration::handle(cmd, context, output_format).await?;
        }

        Commands::Context(cmd) => {
            commands::context::handle(cmd, output_format).await?;
        }

        Commands::System(cmd) => {
            commands::system::handle(cmd, context, output_format).await?;
        }

        Commands::Debug(cmd) => {
            commands::debug::handle(cmd, context, output_format).await?;
        }
    }

    Ok(())
}
