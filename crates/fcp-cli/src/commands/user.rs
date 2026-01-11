//! User command handlers

use crate::api::{ApiClient, CreateUserRequest, UpdatePasswordRequest, UpdateUserRequest};
use crate::config::ResolvedContext;
use crate::{OutputFormat, UserCommands};
use anyhow::{Context as _, Result};
use comfy_table::{presets::UTF8_FULL, ContentArrangement, Table};
use std::io::{self, Write};

/// Handle user commands
pub async fn handle(
    cmd: UserCommands,
    context: ResolvedContext,
    output_format: OutputFormat,
) -> Result<()> {
    let client = ApiClient::new(context.url, context.token)?;

    match cmd {
        UserCommands::List => {
            let users = client
                .list_users()
                .await
                .context("Failed to list users")?;

            match output_format {
                OutputFormat::Table => {
                    let mut table = Table::new();
                    table
                        .load_preset(UTF8_FULL)
                        .set_content_arrangement(ContentArrangement::Dynamic)
                        .set_header(vec!["Username", "Display Name", "Tenant", "Enabled"]);

                    for user in &users {
                        table.add_row(vec![
                            user.username.clone(),
                            user.display_name.clone().unwrap_or_else(|| "-".to_string()),
                            user.tenant_id.clone(),
                            if user.enabled { "yes".to_string() } else { "no".to_string() },
                        ]);
                    }

                    println!("{table}");
                    println!("\nTotal users: {}", users.len());
                }
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&users)?);
                }
                OutputFormat::Yaml => {
                    println!("{}", serde_yaml::to_string(&users)?);
                }
            }
        }

        UserCommands::Show { username } => {
            let user = client
                .get_user(&username)
                .await
                .context(format!("Failed to get user '{}'", username))?;

            match output_format {
                OutputFormat::Table => {
                    println!("User: {}", user.username);
                    println!("  Display Name: {}", user.display_name.unwrap_or_else(|| "-".to_string()));
                    println!("  Tenant ID: {}", user.tenant_id);
                    println!("  Enabled: {}", if user.enabled { "yes" } else { "no" });
                    println!("  Created: {:?}", user.created_at);
                    println!("  Updated: {:?}", user.updated_at);
                }
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&user)?);
                }
                OutputFormat::Yaml => {
                    println!("{}", serde_yaml::to_string(&user)?);
                }
            }
        }

        UserCommands::Add {
            username,
            password,
            display_name,
            tenant_id,
            enabled,
            email,
            phone_number,
            mobile_number,
            pin,
            extension,
            department,
            job_title,
            location,
            language,
            timezone,
            account_status,
            emergency_contact_name,
            emergency_contact_phone,
        } => {
            // Get password (prompt if not provided)
            let password = if let Some(pwd) = password {
                pwd
            } else {
                prompt_password("Enter password: ")?
            };

            let req = CreateUserRequest {
                username: username.clone(),
                password,
                display_name,
                tenant_id,
                enabled: Some(enabled),
                // Enterprise fields - now supported via CLI
                email,
                phone_number,
                mobile_number,
                pin,
                extension,
                department,
                job_title,
                location,
                language,
                timezone,
                account_status,
                emergency_contact_name,
                emergency_contact_phone,
            };

            let user = client
                .create_user(&req)
                .await
                .context(format!("Failed to create user '{}'", username))?;

            match output_format {
                OutputFormat::Table => {
                    println!("User '{}' created successfully", user.username);
                }
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&user)?);
                }
                OutputFormat::Yaml => {
                    println!("{}", serde_yaml::to_string(&user)?);
                }
            }
        }

        UserCommands::Modify {
            username,
            display_name,
            enabled,
            email,
            phone_number,
            mobile_number,
            pin,
            extension,
            department,
            job_title,
            location,
            language,
            timezone,
            account_status,
            emergency_contact_name,
            emergency_contact_phone,
        } => {
            // Check if any fields are being updated
            if display_name.is_none()
                && enabled.is_none()
                && email.is_none()
                && phone_number.is_none()
                && mobile_number.is_none()
                && pin.is_none()
                && extension.is_none()
                && department.is_none()
                && job_title.is_none()
                && location.is_none()
                && language.is_none()
                && timezone.is_none()
                && account_status.is_none()
                && emergency_contact_name.is_none()
                && emergency_contact_phone.is_none()
            {
                return Err(anyhow::anyhow!(
                    "No fields to update. Specify at least one field to modify."
                ));
            }

            let req = UpdateUserRequest {
                display_name,
                enabled,
                // Enterprise fields - now supported via CLI
                email,
                phone_number,
                mobile_number,
                pin,
                extension,
                department,
                job_title,
                location,
                language,
                timezone,
                account_status,
                emergency_contact_name,
                emergency_contact_phone,
            };

            let user = client
                .update_user(&username, &req)
                .await
                .context(format!("Failed to update user '{}'", username))?;

            match output_format {
                OutputFormat::Table => {
                    println!("User '{}' updated successfully", user.username);
                }
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&user)?);
                }
                OutputFormat::Yaml => {
                    println!("{}", serde_yaml::to_string(&user)?);
                }
            }
        }

        UserCommands::Password {
            username,
            new_password,
        } => {
            // Get password (prompt if not provided)
            let password = if let Some(pwd) = new_password {
                pwd
            } else {
                let pwd1 = prompt_password("Enter new password: ")?;
                let pwd2 = prompt_password("Confirm new password: ")?;
                if pwd1 != pwd2 {
                    return Err(anyhow::anyhow!("Passwords do not match"));
                }
                pwd1
            };

            let req = UpdatePasswordRequest { password };

            client
                .update_password(&username, &req)
                .await
                .context(format!("Failed to update password for user '{}'", username))?;

            println!("Password updated successfully for user '{}'", username);
        }

        UserCommands::Delete { username, yes } => {
            // Confirm deletion unless --yes was provided
            if !yes {
                print!("Are you sure you want to delete user '{}'? [y/N]: ", username);
                io::stdout().flush()?;

                let mut input = String::new();
                io::stdin().read_line(&mut input)?;

                if !input.trim().eq_ignore_ascii_case("y") {
                    println!("Deletion cancelled");
                    return Ok(());
                }
            }

            client
                .delete_user(&username)
                .await
                .context(format!("Failed to delete user '{}'", username))?;

            println!("User '{}' deleted successfully", username);
        }
    }

    Ok(())
}

/// Prompt for password from stdin
/// Note: Password will be visible. For production use, consider adding rpassword crate.
fn prompt_password(prompt: &str) -> Result<String> {
    print!("{}", prompt);
    io::stdout().flush()?;

    let mut password = String::new();
    io::stdin()
        .read_line(&mut password)
        .context("Failed to read password")?;
    Ok(password.trim().to_string())
}
