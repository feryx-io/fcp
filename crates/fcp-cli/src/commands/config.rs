//! Configuration command handlers

use crate::{ConfigCommands, OutputFormat};
use anyhow::Result;
use fcp_core::DeploymentMode;

/// Handle config commands
pub async fn handle(cmd: ConfigCommands, _output_format: OutputFormat) -> Result<()> {
    match cmd {
        ConfigCommands::Show => {
            println!("Current FCP Configuration:");
            println!("{}", "=".repeat(60));
            println!();

            // TODO: Load actual config from file
            println!("deployment.mode = enterprise (default)");
            println!();
            println!("Note: Configuration persistence not yet implemented");
            println!("      Use 'fcp config set' to change values (in-memory only for now)");
        }

        ConfigCommands::Set { key, value } => match key.as_str() {
            "deployment.mode" | "deployment_mode" => {
                let mode = parse_deployment_mode(&value)?;
                println!("Would set deployment mode to: {}", mode.name());
                println!();
                println!("Note: Configuration persistence not yet implemented");
                println!("      This change would only affect the current session");
            }
            _ => {
                anyhow::bail!(
                    "Unknown configuration key '{}'. Valid keys: deployment.mode",
                    key
                );
            }
        },

        ConfigCommands::Get { key } => {
            match key.as_str() {
                "deployment.mode" | "deployment_mode" => {
                    // TODO: Load from actual config
                    println!("deployment.mode = enterprise (default)");
                }
                _ => {
                    anyhow::bail!(
                        "Unknown configuration key '{}'. Valid keys: deployment.mode",
                        key
                    );
                }
            }
        }
    }

    Ok(())
}

/// Parse deployment mode from string
fn parse_deployment_mode(mode_str: &str) -> Result<DeploymentMode> {
    match mode_str.to_lowercase().as_str() {
        "carrier" => Ok(DeploymentMode::Carrier),
        "service_provider" | "serviceprovider" => Ok(DeploymentMode::ServiceProvider),
        "enterprise" => Ok(DeploymentMode::Enterprise),
        _ => anyhow::bail!(
            "Invalid deployment mode '{}'. Valid modes: carrier, service_provider, enterprise",
            mode_str
        ),
    }
}
