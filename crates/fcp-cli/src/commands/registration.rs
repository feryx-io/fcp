//! Registration command handlers

use crate::api::ApiClient;
use crate::config::ResolvedContext;
use crate::{OutputFormat, RegistrationCommands};
use anyhow::{Context as _, Result};
use comfy_table::{presets::UTF8_FULL, ContentArrangement, Table};
use std::io::{self, Write};

/// Handle registration commands
pub async fn handle(
    cmd: RegistrationCommands,
    context: ResolvedContext,
    output_format: OutputFormat,
) -> Result<()> {
    let client = ApiClient::new(context.url, context.token)?;

    match cmd {
        RegistrationCommands::List { aor } => {
            // Normalize AOR (add sip: prefix if missing)
            let normalized_aor = normalize_aor(&aor);

            let registrations = client
                .list_registrations(&normalized_aor)
                .await
                .context(format!("Failed to list registrations for '{}'", normalized_aor))?;

            match output_format {
                OutputFormat::Table => {
                    if registrations.is_empty() {
                        println!("No active registrations for '{}'", normalized_aor);
                        return Ok(());
                    }

                    let mut table = Table::new();
                    table
                        .load_preset(UTF8_FULL)
                        .set_content_arrangement(ContentArrangement::Dynamic)
                        .set_header(vec![
                            "Contact",
                            "User Agent",
                            "Expires In",
                            "Q-Value",
                        ]);

                    for reg in &registrations {
                        let user_agent = reg.user_agent.as_deref().unwrap_or("-");
                        let expires_in = format_expires_in(reg.expires_in_seconds);
                        let q_value = match reg.q {
                            Some(q) => format!("{:.2}", q),
                            None => "-".to_string(),
                        };

                        table.add_row(vec![
                            reg.contact.clone(),
                            user_agent.to_string(),
                            expires_in,
                            q_value,
                        ]);
                    }

                    println!("Registrations for '{}':\n", normalized_aor);
                    println!("{table}");
                    println!("\nTotal registrations: {}", registrations.len());
                }
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&registrations)?);
                }
                OutputFormat::Yaml => {
                    println!("{}", serde_yaml::to_string(&registrations)?);
                }
            }
        }

        RegistrationCommands::Delete {
            aor,
            contact,
            yes,
        } => {
            // Normalize AOR (add sip: prefix if missing)
            let normalized_aor = normalize_aor(&aor);

            match contact {
                Some(contact_uri) => {
                    // Delete specific registration
                    if !yes {
                        print!(
                            "Are you sure you want to delete registration for '{}' from '{}'? [y/N]: ",
                            contact_uri, normalized_aor
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
                        .delete_registration(&normalized_aor, &contact_uri)
                        .await
                        .context(format!(
                            "Failed to delete registration for '{}' from '{}'",
                            contact_uri, normalized_aor
                        ))?;

                    println!(
                        "Registration for '{}' from '{}' deleted successfully",
                        contact_uri, normalized_aor
                    );
                }
                None => {
                    // Delete all registrations for AOR
                    if !yes {
                        print!(
                            "Are you sure you want to delete ALL registrations for '{}'? [y/N]: ",
                            normalized_aor
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
                        .delete_registrations(&normalized_aor)
                        .await
                        .context(format!(
                            "Failed to delete all registrations for '{}'",
                            normalized_aor
                        ))?;

                    println!(
                        "All registrations for '{}' deleted successfully",
                        normalized_aor
                    );
                }
            }
        }
    }

    Ok(())
}

/// Normalize AOR by adding sip: prefix if missing
fn normalize_aor(aor: &str) -> String {
    if aor.starts_with("sip:") || aor.starts_with("sips:") {
        aor.to_string()
    } else {
        format!("sip:{}", aor)
    }
}

/// Format expires_in_seconds to human-readable form
fn format_expires_in(seconds: u64) -> String {
    if seconds == 0 {
        "expired".to_string()
    } else if seconds < 60 {
        format!("{}s", seconds)
    } else if seconds < 3600 {
        format!("{}m {}s", seconds / 60, seconds % 60)
    } else {
        let hours = seconds / 3600;
        let minutes = (seconds % 3600) / 60;
        format!("{}h {}m", hours, minutes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_aor() {
        assert_eq!(normalize_aor("alice@example.com"), "sip:alice@example.com");
        assert_eq!(normalize_aor("sip:bob@example.com"), "sip:bob@example.com");
        assert_eq!(normalize_aor("sips:charlie@example.com"), "sips:charlie@example.com");
    }

    #[test]
    fn test_format_expires_in() {
        assert_eq!(format_expires_in(0), "expired");
        assert_eq!(format_expires_in(30), "30s");
        assert_eq!(format_expires_in(90), "1m 30s");
        assert_eq!(format_expires_in(3661), "1h 1m");
        assert_eq!(format_expires_in(7200), "2h 0m");
    }
}
