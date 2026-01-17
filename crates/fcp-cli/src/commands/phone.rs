//! Phone number command handlers

use crate::api::ApiClient;
use crate::config::ResolvedContext;
use crate::{OutputFormat, PhoneCommands};
use anyhow::{Context as _, Result};
use comfy_table::{presets::UTF8_FULL, ContentArrangement, Table};

/// Handle phone number commands
pub async fn handle(
    cmd: PhoneCommands,
    context: ResolvedContext,
    output_format: OutputFormat,
) -> Result<()> {
    let client = ApiClient::new(context.url, context.token)?;

    match cmd {
        PhoneCommands::List { tenant_id } => {
            let phones = client
                .list_phone_numbers(tenant_id.as_deref())
                .await
                .context("Failed to list phone numbers")?;

            match output_format {
                OutputFormat::Table => {
                    let mut table = Table::new();
                    table
                        .load_preset(UTF8_FULL)
                        .set_content_arrangement(ContentArrangement::Dynamic)
                        .set_header(vec![
                            "ID",
                            "Number",
                            "State",
                            "CNAM",
                            "E911",
                            "Tenant",
                        ]);

                    for phone in &phones {
                        table.add_row(vec![
                            phone.id.clone(),
                            phone.e164.clone(),
                            phone.state.clone(),
                            phone.cnam.clone().unwrap_or_else(|| "-".to_string()),
                            if phone.e911_address.is_some() {
                                "configured"
                            } else {
                                "-"
                            }
                            .to_string(),
                            phone.tenant_id.clone(),
                        ]);
                    }

                    println!("{table}");
                    println!("\nTotal phone numbers: {}", phones.len());
                }
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&phones)?);
                }
                OutputFormat::Yaml => {
                    println!("{}", serde_yaml::to_string(&phones)?);
                }
            }
        }

        PhoneCommands::Show { id } => {
            let phone = client
                .get_phone_number(&id)
                .await
                .context(format!("Failed to get phone number '{}'", id))?;

            match output_format {
                OutputFormat::Table => {
                    println!("Phone Number: {}", phone.e164);
                    println!("  ID: {}", phone.id);
                    println!("  State: {}", phone.state);
                    println!("  Country Code: {}", phone.country_code);
                    println!("  NPA: {}", phone.npa);
                    println!("  NXX: {}", phone.nxx);
                    println!("  Line: {}", phone.line);
                    println!(
                        "  CNAM: {}",
                        phone.cnam.unwrap_or_else(|| "-".to_string())
                    );
                    if let Some(e911) = &phone.e911_address {
                        println!("  E911 Address:");
                        println!("    Address: {}", e911.address1);
                        if let Some(addr2) = &e911.address2 {
                            println!("             {}", addr2);
                        }
                        println!("    City: {}", e911.city);
                        println!("    State: {}", e911.state);
                        println!("    Postal Code: {}", e911.postal_code);
                        println!("    Country: {}", e911.country);
                        println!("    Validated: {}", e911.validated);
                    } else {
                        println!("  E911 Address: Not configured");
                    }
                    println!(
                        "  E911 Callback: {}",
                        phone.e911_callback.unwrap_or_else(|| "-".to_string())
                    );
                    println!("  Recording Enabled: {}", phone.recording_enabled);
                    println!("  Block Anonymous: {}", phone.block_anonymous);
                    println!("  Tenant ID: {}", phone.tenant_id);
                    println!("  Created: {:?}", phone.created_at);
                    println!("  Updated: {:?}", phone.updated_at);
                }
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&phone)?);
                }
                OutputFormat::Yaml => {
                    println!("{}", serde_yaml::to_string(&phone)?);
                }
            }
        }

        PhoneCommands::Add {
            e164,
            tenant_id,
            cnam,
        } => {
            #[derive(serde::Serialize)]
            struct CreatePhoneNumberRequest {
                e164: String,
                tenant_id: String,
                #[serde(skip_serializing_if = "Option::is_none")]
                cnam: Option<String>,
            }

            let request = CreatePhoneNumberRequest {
                e164,
                tenant_id,
                cnam,
            };

            let phone = client
                .create_phone_number(&request)
                .await
                .context("Failed to create phone number")?;

            println!("✓ Phone number created: {} ({})", phone.e164, phone.id);
            println!("  State: {}", phone.state);
            println!(
                "  Note: Use 'fcp phone activate {}' to activate after configuring E911",
                phone.id
            );
        }

        PhoneCommands::Update {
            id,
            cnam,
            e911_street,
            e911_city,
            e911_state,
            e911_postal_code,
            e911_country,
            e911_callback,
            recording_enabled,
            block_anonymous,
        } => {
            #[derive(serde::Serialize)]
            struct UpdatePhoneNumberRequest {
                #[serde(skip_serializing_if = "Option::is_none")]
                cnam: Option<String>,
                #[serde(skip_serializing_if = "Option::is_none")]
                e911_address: Option<E911AddressRequest>,
                #[serde(skip_serializing_if = "Option::is_none")]
                e911_callback: Option<String>,
                #[serde(skip_serializing_if = "Option::is_none")]
                recording_enabled: Option<bool>,
                #[serde(skip_serializing_if = "Option::is_none")]
                block_anonymous: Option<bool>,
            }

            #[derive(serde::Serialize)]
            struct E911AddressRequest {
                street: String,
                city: String,
                state: String,
                postal_code: String,
                country: String,
            }

            let e911_address = if e911_street.is_some()
                || e911_city.is_some()
                || e911_state.is_some()
                || e911_postal_code.is_some()
                || e911_country.is_some()
            {
                // If any E911 field is provided, all are required
                Some(E911AddressRequest {
                    street: e911_street.ok_or_else(|| {
                        anyhow::anyhow!("E911 address requires all fields: --e911-street")
                    })?,
                    city: e911_city.ok_or_else(|| {
                        anyhow::anyhow!("E911 address requires all fields: --e911-city")
                    })?,
                    state: e911_state.ok_or_else(|| {
                        anyhow::anyhow!("E911 address requires all fields: --e911-state")
                    })?,
                    postal_code: e911_postal_code.ok_or_else(|| {
                        anyhow::anyhow!("E911 address requires all fields: --e911-postal-code")
                    })?,
                    country: e911_country.unwrap_or_else(|| "US".to_string()),
                })
            } else {
                None
            };

            let request = UpdatePhoneNumberRequest {
                cnam,
                e911_address,
                e911_callback,
                recording_enabled,
                block_anonymous,
            };

            let phone = client
                .update_phone_number(&id, &request)
                .await
                .context(format!("Failed to update phone number '{}'", id))?;

            println!("✓ Phone number updated: {} ({})", phone.e164, phone.id);
        }

        PhoneCommands::Delete { id, force } => {
            if !force {
                print!("Are you sure you want to delete phone number '{}'? [y/N] ", id);
                std::io::Write::flush(&mut std::io::stdout())?;

                let mut input = String::new();
                std::io::stdin().read_line(&mut input)?;

                if !input.trim().eq_ignore_ascii_case("y") {
                    println!("Cancelled.");
                    return Ok(());
                }
            }

            client
                .delete_phone_number(&id)
                .await
                .context(format!("Failed to delete phone number '{}'", id))?;

            println!("✓ Phone number deleted: {}", id);
        }

        PhoneCommands::Activate { id } => {
            let phone = client
                .activate_phone_number(&id)
                .await
                .context(format!("Failed to activate phone number '{}'", id))?;

            println!("✓ Phone number activated: {} ({})", phone.e164, phone.id);
            println!("  State: {}", phone.state);
        }
    }

    Ok(())
}
