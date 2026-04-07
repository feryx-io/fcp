//! User command handlers

use crate::api::{ApiClient, CreateUserRequest, UpdatePasswordRequest, UpdateUserRequest};
use crate::config::ResolvedContext;
use crate::{OutputFormat, UserCommands};
use anyhow::{Context as _, Result};
use comfy_table::{presets::UTF8_FULL, ContentArrangement, Table};
use fcp_core::{DeploymentMode, Permission, UserRole};
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
            let users = client.list_users().await.context("Failed to list users")?;

            match output_format {
                OutputFormat::Table => {
                    let mut table = Table::new();
                    table
                        .load_preset(UTF8_FULL)
                        .set_content_arrangement(ContentArrangement::Dynamic)
                        .set_header(vec![
                            "Username",
                            "Display Name",
                            "Role",
                            "Tenant",
                            "Enabled",
                        ]);

                    for user in &users {
                        table.add_row(vec![
                            user.username.clone(),
                            user.display_name.clone().unwrap_or_else(|| "-".to_string()),
                            user.role.clone(),
                            user.tenant_id.clone(),
                            if user.enabled {
                                "yes".to_string()
                            } else {
                                "no".to_string()
                            },
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
                    println!(
                        "  Display Name: {}",
                        user.display_name.as_deref().unwrap_or("-")
                    );
                    println!("  Role: {}", user.role);
                    if let Some(dept_id) = &user.department_id {
                        println!("  Department ID: {}", dept_id);
                    }
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
            role,
            department_id,
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
            // Validate role if provided
            let role_str = if let Some(ref r) = role {
                let user_role = parse_role(r)?;
                Some(role_to_api_string(&user_role))
            } else {
                None
            };

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
                role: role_str,
                department_id,
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
            role,
            department_id,
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
                && role.is_none()
                && department_id.is_none()
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

            // Validate role if provided
            let role_str = if let Some(ref r) = role {
                let user_role = parse_role(r)?;
                Some(role_to_api_string(&user_role))
            } else {
                None
            };

            let req = UpdateUserRequest {
                display_name,
                enabled,
                role: role_str,
                department_id,
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
                print!(
                    "Are you sure you want to delete user '{}'? [y/N]: ",
                    username
                );
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

        UserCommands::Create {
            username,
            role,
            display_name,
            email,
            department,
            tenant_id,
        } => {
            // Parse role from string
            let user_role = parse_role(&role)?;

            // Create user with role
            let create_request = CreateUserRequest {
                username: username.clone(),
                password: "changeme".to_string(), // Default password
                display_name: display_name.clone(),
                tenant_id,
                enabled: Some(true),
                role: Some(role_to_api_string(&user_role)),
                department_id: None,
                // Set email and department
                email,
                department,
                // Default other fields
                phone_number: None,
                mobile_number: None,
                pin: None,
                extension: None,
                job_title: None,
                location: None,
                language: None,
                timezone: None,
                account_status: None,
                emergency_contact_name: None,
                emergency_contact_phone: None,
            };

            client
                .create_user(&create_request)
                .await
                .context("Failed to create user")?;

            println!(
                "User '{}' created successfully with role: {}",
                username,
                user_role.name()
            );
            println!(
                "Note: Use 'fcp user password {}' to set a password",
                username
            );
        }

        UserCommands::Update {
            username,
            role,
            department,
        } => {
            if role.is_none() && department.is_none() {
                anyhow::bail!("At least one of --role or --department must be provided");
            }

            // Validate role if provided
            let role_str = if let Some(ref r) = role {
                let user_role = parse_role(r)?;
                Some(role_to_api_string(&user_role))
            } else {
                None
            };

            let update_request = UpdateUserRequest {
                display_name: None,
                enabled: None,
                role: role_str,
                department_id: department.clone(),
                email: None,
                phone_number: None,
                mobile_number: None,
                pin: None,
                extension: None,
                department: None,
                job_title: None,
                location: None,
                language: None,
                timezone: None,
                account_status: None,
                emergency_contact_name: None,
                emergency_contact_phone: None,
            };

            let user = client
                .update_user(&username, &update_request)
                .await
                .context("Failed to update user")?;

            match output_format {
                OutputFormat::Table => {
                    if role.is_some() {
                        println!("User '{}' role updated to: {}", username, user.role);
                    }
                    if let Some(dept) = &department {
                        println!("User '{}' department updated to: {}", username, dept);
                    }
                }
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&user)?);
                }
                OutputFormat::Yaml => {
                    println!("{}", serde_yaml::to_string(&user)?);
                }
            }
        }

        UserCommands::Grant {
            username,
            permission,
        } => {
            let perm = parse_permission(&permission)?;
            println!(
                "Would grant permission '{}' to user '{}'",
                perm.name(),
                username
            );
            println!(
                "Note: Permission grant/revoke requires storing per-user permissions overrides"
            );
            println!("      This feature is not yet implemented in the storage layer");
        }

        UserCommands::Revoke {
            username,
            permission,
        } => {
            let perm = parse_permission(&permission)?;
            println!(
                "Would revoke permission '{}' from user '{}'",
                perm.name(),
                username
            );
            println!(
                "Note: Permission grant/revoke requires storing per-user permissions overrides"
            );
            println!("      This feature is not yet implemented in the storage layer");
        }

        UserCommands::ListPermissions { username } => {
            let user = client
                .get_user(&username)
                .await
                .context(format!("Failed to get user '{}'", username))?;

            let user_role = parse_role(&user.role).unwrap_or(UserRole::EndUser);

            // Default to Enterprise mode for now
            let deployment_mode = DeploymentMode::Enterprise;

            let permissions = user_role.permissions(deployment_mode);

            println!("User: {}", username);
            println!("Role: {} ({})", user_role.name(), user.role);
            println!("Deployment mode: {}", deployment_mode.name());
            println!();
            println!("Permissions ({}):", permissions.len());
            println!("{}", "-".repeat(60));

            for perm in &permissions {
                println!("  - {} ({})", format!("{:?}", perm), perm.name());
            }
        }
    }

    Ok(())
}

/// Convert a UserRole to its API-compatible snake_case string
fn role_to_api_string(role: &UserRole) -> String {
    serde_json::to_string(role)
        .unwrap_or_else(|_| "\"end_user\"".to_string())
        .trim_matches('"')
        .to_string()
}

/// Parse a role string to UserRole enum
fn parse_role(role_str: &str) -> Result<UserRole> {
    match role_str.to_lowercase().as_str() {
        "super_admin" | "superadmin" => Ok(UserRole::SuperAdmin),
        "telecom_admin" | "telecomadmin" => Ok(UserRole::TelecomAdmin),
        "user_admin" | "useradmin" => Ok(UserRole::UserAdmin),
        "department_admin" | "departmentadmin" => Ok(UserRole::DepartmentAdmin),
        "read_only" | "readonly" => Ok(UserRole::ReadOnly),
        "end_user" | "enduser" => Ok(UserRole::EndUser),
        _ => anyhow::bail!(
            "Invalid role '{}'. Valid roles: super_admin, telecom_admin, user_admin, department_admin, read_only, end_user",
            role_str
        ),
    }
}

/// Parse a permission string to Permission enum
fn parse_permission(perm_str: &str) -> Result<Permission> {
    match perm_str.to_lowercase().as_str() {
        "system_config" => Ok(Permission::SystemConfig),
        "system_view_logs" => Ok(Permission::SystemViewLogs),
        "user_create" => Ok(Permission::UserCreate),
        "user_read" => Ok(Permission::UserRead),
        "user_update" => Ok(Permission::UserUpdate),
        "user_delete" => Ok(Permission::UserDelete),
        "user_set_password" => Ok(Permission::UserSetPassword),
        "phone_number_create" => Ok(Permission::PhoneNumberCreate),
        "phone_number_read" => Ok(Permission::PhoneNumberRead),
        "phone_number_update" => Ok(Permission::PhoneNumberUpdate),
        "phone_number_delete" => Ok(Permission::PhoneNumberDelete),
        "phone_number_view_carrier_data" => Ok(Permission::PhoneNumberViewCarrierData),
        "device_create" => Ok(Permission::DeviceCreate),
        "device_read" => Ok(Permission::DeviceRead),
        "device_update" => Ok(Permission::DeviceUpdate),
        "device_delete" => Ok(Permission::DeviceDelete),
        "call_view" => Ok(Permission::CallView),
        "call_control" => Ok(Permission::CallControl),
        "call_recording_access" => Ok(Permission::CallRecordingAccess),
        "registration_view" => Ok(Permission::RegistrationView),
        "registration_manage" => Ok(Permission::RegistrationManage),
        "trunk_create" => Ok(Permission::TrunkCreate),
        "trunk_read" => Ok(Permission::TrunkRead),
        "trunk_update" => Ok(Permission::TrunkUpdate),
        "trunk_delete" => Ok(Permission::TrunkDelete),
        "tenant_create" => Ok(Permission::TenantCreate),
        "tenant_read" => Ok(Permission::TenantRead),
        "tenant_update" => Ok(Permission::TenantUpdate),
        "tenant_delete" => Ok(Permission::TenantDelete),
        "debug_view" => Ok(Permission::DebugView),
        _ => anyhow::bail!("Invalid permission '{}'. See 'fcp users list-permissions <username>' for valid permissions", perm_str),
    }
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
