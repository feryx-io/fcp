//! System command handlers

use crate::api::ApiClient;
use crate::config::ResolvedContext;
use crate::OutputFormat;
use anyhow::Result;

pub async fn handle(
    cmd: crate::SystemCommands,
    context: ResolvedContext,
    output_format: OutputFormat,
) -> Result<()> {
    let client = ApiClient::new(context.url, context.token)?;

    match cmd {
        crate::SystemCommands::Health => {
            let health = client.get_health().await?;
            match output_format {
                OutputFormat::Json => crate::output::print_json(&health),
                OutputFormat::Yaml => crate::output::print_yaml(&health),
                OutputFormat::Table => {
                    println!("Server Health: {}", health.status);
                    println!("Version: {}", health.version);
                }
            }
        }
        crate::SystemCommands::Status => {
            let status = client.get_status().await?;
            match output_format {
                OutputFormat::Json => crate::output::print_json(&status),
                OutputFormat::Yaml => crate::output::print_yaml(&status),
                OutputFormat::Table => {
                    println!("Server Status: {}", status.status);
                    println!("Version: {}", status.version);
                    println!("Uptime: {} seconds", status.uptime_seconds);
                    println!("Users: {}", status.users_count);
                    println!("Active Registrations: {}", status.active_registrations_count);
                }
            }
        }
    }

    Ok(())
}
