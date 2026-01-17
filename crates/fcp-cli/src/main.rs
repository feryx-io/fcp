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

    /// Phone number management commands
    #[command(subcommand)]
    Phone(PhoneCommands),

    /// Device management commands
    #[command(subcommand)]
    Device(DeviceCommands),
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

        // ===== Enterprise fields =====
        /// Email address
        #[arg(long)]
        email: Option<String>,

        /// Phone number
        #[arg(long)]
        phone_number: Option<String>,

        /// Mobile number
        #[arg(long)]
        mobile_number: Option<String>,

        /// PIN (for voicemail)
        #[arg(long)]
        pin: Option<String>,

        /// Extension number
        #[arg(long)]
        extension: Option<String>,

        /// Department
        #[arg(long)]
        department: Option<String>,

        /// Job title
        #[arg(long)]
        job_title: Option<String>,

        /// Location/Office
        #[arg(long)]
        location: Option<String>,

        /// Language code (default: en-US)
        #[arg(long)]
        language: Option<String>,

        /// Timezone (default: UTC)
        #[arg(long)]
        timezone: Option<String>,

        /// Account status (default: active)
        #[arg(long)]
        account_status: Option<String>,

        /// Emergency contact name
        #[arg(long)]
        emergency_contact_name: Option<String>,

        /// Emergency contact phone
        #[arg(long)]
        emergency_contact_phone: Option<String>,
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

        // ===== Enterprise fields =====
        /// Email address
        #[arg(long)]
        email: Option<String>,

        /// Phone number
        #[arg(long)]
        phone_number: Option<String>,

        /// Mobile number
        #[arg(long)]
        mobile_number: Option<String>,

        /// PIN (for voicemail)
        #[arg(long)]
        pin: Option<String>,

        /// Extension number
        #[arg(long)]
        extension: Option<String>,

        /// Department
        #[arg(long)]
        department: Option<String>,

        /// Job title
        #[arg(long)]
        job_title: Option<String>,

        /// Location/Office
        #[arg(long)]
        location: Option<String>,

        /// Language code
        #[arg(long)]
        language: Option<String>,

        /// Timezone
        #[arg(long)]
        timezone: Option<String>,

        /// Account status
        #[arg(long)]
        account_status: Option<String>,

        /// Emergency contact name
        #[arg(long)]
        emergency_contact_name: Option<String>,

        /// Emergency contact phone
        #[arg(long)]
        emergency_contact_phone: Option<String>,
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

#[derive(Debug, Subcommand)]
pub enum PhoneCommands {
    /// List all phone numbers
    List {
        /// Filter by tenant ID
        #[arg(long)]
        tenant_id: Option<String>,
    },

    /// Show detailed information for a phone number
    Show {
        /// Phone number ID
        id: String,
    },

    /// Add a new phone number
    Add {
        /// E.164 formatted number (e.g., +15551234567)
        e164: String,

        /// Tenant ID
        #[arg(long)]
        tenant_id: String,

        /// CNAM (Caller ID name)
        #[arg(long)]
        cnam: Option<String>,
    },

    /// Update a phone number
    Update {
        /// Phone number ID
        id: String,

        /// CNAM (Caller ID name)
        #[arg(long)]
        cnam: Option<String>,

        /// E911 street address
        #[arg(long)]
        e911_street: Option<String>,

        /// E911 city
        #[arg(long)]
        e911_city: Option<String>,

        /// E911 state
        #[arg(long)]
        e911_state: Option<String>,

        /// E911 postal code
        #[arg(long)]
        e911_postal_code: Option<String>,

        /// E911 country (default: US)
        #[arg(long)]
        e911_country: Option<String>,

        /// E911 callback number
        #[arg(long)]
        e911_callback: Option<String>,

        /// Enable/disable call recording
        #[arg(long)]
        recording_enabled: Option<bool>,

        /// Enable/disable blocking anonymous calls
        #[arg(long)]
        block_anonymous: Option<bool>,
    },

    /// Delete a phone number
    Delete {
        /// Phone number ID
        id: String,

        /// Skip confirmation prompt
        #[arg(short = 'y', long)]
        force: bool,
    },

    /// Activate a phone number (requires E911)
    Activate {
        /// Phone number ID
        id: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum DeviceCommands {
    /// List all devices
    List {
        /// Filter by tenant ID
        #[arg(long)]
        tenant_id: Option<String>,

        /// Filter by user ID
        #[arg(long)]
        user_id: Option<String>,
    },

    /// Show detailed information for a device
    Show {
        /// Device ID
        id: String,
    },

    /// Add a new device
    Add {
        /// Device name
        name: String,

        /// Device type (e.g., desk_phone, softphone, webrtc, mobile, gateway)
        #[arg(long)]
        device_type: String,

        /// Tenant ID
        #[arg(long)]
        tenant_id: String,

        /// Owner user ID (optional)
        #[arg(long)]
        owner_user_id: Option<String>,

        /// MAC address
        #[arg(long)]
        mac_address: Option<String>,

        /// Model name
        #[arg(long)]
        model: Option<String>,

        /// SIP URI
        #[arg(long)]
        sip_uri: Option<String>,

        /// Location
        #[arg(long)]
        location: Option<String>,
    },

    /// Update a device
    Update {
        /// Device ID
        id: String,

        /// Device name
        #[arg(long)]
        name: Option<String>,

        /// Enable/disable device
        #[arg(long)]
        enabled: Option<bool>,

        /// Owner user ID
        #[arg(long)]
        owner_user_id: Option<String>,

        /// MAC address
        #[arg(long)]
        mac_address: Option<String>,

        /// Model name
        #[arg(long)]
        model: Option<String>,

        /// Firmware version
        #[arg(long)]
        firmware_version: Option<String>,

        /// SIP URI
        #[arg(long)]
        sip_uri: Option<String>,

        /// Location
        #[arg(long)]
        location: Option<String>,
    },

    /// Delete a device
    Delete {
        /// Device ID
        id: String,

        /// Skip confirmation prompt
        #[arg(short = 'y', long)]
        force: bool,
    },
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

        Commands::Phone(cmd) => {
            commands::phone::handle(cmd, context, output_format).await?;
        }

        Commands::Device(cmd) => {
            commands::device::handle(cmd, context, output_format).await?;
        }
    }

    Ok(())
}
