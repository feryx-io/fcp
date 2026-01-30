//! Device command handlers

use crate::api::ApiClient;
use crate::config::ResolvedContext;
use crate::{DeviceCommands, OutputFormat};
use anyhow::{Context as _, Result};
use comfy_table::{presets::UTF8_FULL, ContentArrangement, Table};

/// Handle device commands
pub async fn handle(
    cmd: DeviceCommands,
    context: ResolvedContext,
    output_format: OutputFormat,
) -> Result<()> {
    let client = ApiClient::new(context.url, context.token)?;

    match cmd {
        DeviceCommands::List { tenant_id, user_id } => {
            let devices = if let Some(user) = user_id {
                client
                    .list_devices_by_user(&user)
                    .await
                    .context("Failed to list devices for user")?
            } else {
                client
                    .list_devices(tenant_id.as_deref())
                    .await
                    .context("Failed to list devices")?
            };

            match output_format {
                OutputFormat::Table => {
                    let mut table = Table::new();
                    table
                        .load_preset(UTF8_FULL)
                        .set_content_arrangement(ContentArrangement::Dynamic)
                        .set_header(vec![
                            "ID", "Type", "MAC", "User", "Model", "Enabled", "Tenant",
                        ]);

                    for device in &devices {
                        table.add_row(vec![
                            device.id.clone(),
                            device.device_type.clone(),
                            device
                                .mac_address
                                .clone()
                                .unwrap_or_else(|| "-".to_string()),
                            device
                                .owner_user_id
                                .clone()
                                .unwrap_or_else(|| "-".to_string()),
                            device.model.clone().unwrap_or_else(|| "-".to_string()),
                            if device.enabled { "yes" } else { "no" }.to_string(),
                            device.tenant_id.clone(),
                        ]);
                    }

                    println!("{table}");
                    println!("\nTotal devices: {}", devices.len());
                }
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&devices)?);
                }
                OutputFormat::Yaml => {
                    println!("{}", serde_yaml::to_string(&devices)?);
                }
            }
        }

        DeviceCommands::Show { id } => {
            let device = client
                .get_device(&id)
                .await
                .context(format!("Failed to get device '{}'", id))?;

            match output_format {
                OutputFormat::Table => {
                    println!("Device: {}", device.id);
                    println!("  Type: {}", device.device_type);
                    println!(
                        "  MAC Address: {}",
                        device.mac_address.unwrap_or_else(|| "-".to_string())
                    );
                    println!(
                        "  Owner User ID: {}",
                        device.owner_user_id.unwrap_or_else(|| "-".to_string())
                    );
                    println!(
                        "  Model: {}",
                        device.model.unwrap_or_else(|| "-".to_string())
                    );
                    println!(
                        "  Firmware: {}",
                        device.firmware_version.unwrap_or_else(|| "-".to_string())
                    );
                    println!(
                        "  SIP URI: {}",
                        device.sip_uri.unwrap_or_else(|| "-".to_string())
                    );
                    println!("  Enabled: {}", device.enabled);
                    println!(
                        "  Location: {}",
                        device.location.unwrap_or_else(|| "-".to_string())
                    );
                    println!("  Tenant ID: {}", device.tenant_id);
                    println!("  Created: {:?}", device.created_at);
                    println!("  Updated: {:?}", device.updated_at);
                }
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&device)?);
                }
                OutputFormat::Yaml => {
                    println!("{}", serde_yaml::to_string(&device)?);
                }
            }
        }

        DeviceCommands::Add {
            name,
            device_type,
            tenant_id,
            owner_user_id,
            mac_address,
            model,
            sip_uri,
            location,
        } => {
            #[derive(serde::Serialize)]
            struct CreateDeviceRequest {
                name: String,
                device_type: String,
                tenant_id: String,
                #[serde(skip_serializing_if = "Option::is_none")]
                owner_user_id: Option<String>,
                #[serde(skip_serializing_if = "Option::is_none")]
                mac_address: Option<String>,
                #[serde(skip_serializing_if = "Option::is_none")]
                model: Option<String>,
                #[serde(skip_serializing_if = "Option::is_none")]
                sip_uri: Option<String>,
                #[serde(skip_serializing_if = "Option::is_none")]
                location: Option<String>,
            }

            let request = CreateDeviceRequest {
                name,
                device_type,
                tenant_id,
                owner_user_id,
                mac_address,
                model,
                sip_uri,
                location,
            };

            let device = client
                .create_device(&request)
                .await
                .context("Failed to create device")?;

            println!("✓ Device created: {}", device.id);
            println!("  Type: {}", device.device_type);
            if let Some(mac) = device.mac_address {
                println!("  MAC Address: {}", mac);
            }
        }

        DeviceCommands::Update {
            id,
            name,
            enabled,
            owner_user_id,
            mac_address,
            model,
            firmware_version,
            sip_uri,
            location,
        } => {
            #[derive(serde::Serialize)]
            struct UpdateDeviceRequest {
                #[serde(skip_serializing_if = "Option::is_none")]
                name: Option<String>,
                #[serde(skip_serializing_if = "Option::is_none")]
                enabled: Option<bool>,
                #[serde(skip_serializing_if = "Option::is_none")]
                owner_user_id: Option<String>,
                #[serde(skip_serializing_if = "Option::is_none")]
                mac_address: Option<String>,
                #[serde(skip_serializing_if = "Option::is_none")]
                model: Option<String>,
                #[serde(skip_serializing_if = "Option::is_none")]
                firmware_version: Option<String>,
                #[serde(skip_serializing_if = "Option::is_none")]
                sip_uri: Option<String>,
                #[serde(skip_serializing_if = "Option::is_none")]
                location: Option<String>,
            }

            let request = UpdateDeviceRequest {
                name,
                enabled,
                owner_user_id,
                mac_address,
                model,
                firmware_version,
                sip_uri,
                location,
            };

            let device = client
                .update_device(&id, &request)
                .await
                .context(format!("Failed to update device '{}'", id))?;

            println!("✓ Device updated: {}", device.id);
        }

        DeviceCommands::Delete { id, force } => {
            if !force {
                print!("Are you sure you want to delete device '{}'? [y/N] ", id);
                std::io::Write::flush(&mut std::io::stdout())?;

                let mut input = String::new();
                std::io::stdin().read_line(&mut input)?;

                if !input.trim().eq_ignore_ascii_case("y") {
                    println!("Cancelled.");
                    return Ok(());
                }
            }

            client
                .delete_device(&id)
                .await
                .context(format!("Failed to delete device '{}'", id))?;

            println!("✓ Device deleted: {}", id);
        }
    }

    Ok(())
}
