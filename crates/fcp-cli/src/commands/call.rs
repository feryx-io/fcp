//! Call command handlers

use crate::api::ApiClient;
use crate::config::ResolvedContext;
use crate::{CallCommands, OutputFormat};
use anyhow::{Context as _, Result};
use comfy_table::{presets::UTF8_FULL, ContentArrangement, Table};
use std::io::{self, Write};

/// Handle call commands
pub async fn handle(
    cmd: CallCommands,
    context: ResolvedContext,
    output_format: OutputFormat,
) -> Result<()> {
    let client = ApiClient::new(context.url, context.token)?;

    match cmd {
        CallCommands::List => {
            let calls = client
                .list_calls()
                .await
                .context("Failed to list calls")?;

            match output_format {
                OutputFormat::Table => {
                    if calls.is_empty() {
                        println!("No active calls");
                        return Ok(());
                    }

                    let mut table = Table::new();
                    table
                        .load_preset(UTF8_FULL)
                        .set_content_arrangement(ContentArrangement::Dynamic)
                        .set_header(vec![
                            "Call ID",
                            "State",
                            "From",
                            "To",
                            "Duration",
                            "Media",
                        ]);

                    for call in &calls {
                        let duration = match call.duration_seconds {
                            Some(secs) => format_duration(secs),
                            None => "-".to_string(),
                        };

                        let from = format_party(&call.from_uri, &call.from_display_name);
                        let to = format_party(&call.to_uri, &call.to_display_name);

                        table.add_row(vec![
                            call.id.clone(),
                            call.state.clone(),
                            from,
                            to,
                            duration,
                            if call.media_anchored { "yes" } else { "no" }.to_string(),
                        ]);
                    }

                    println!("{table}");
                    println!("\nTotal active calls: {}", calls.len());
                }
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&calls)?);
                }
                OutputFormat::Yaml => {
                    println!("{}", serde_yaml::to_string(&calls)?);
                }
            }
        }

        CallCommands::Show { call_id } => {
            let call = client
                .get_call(&call_id)
                .await
                .context(format!("Failed to get call '{}'", call_id))?;

            match output_format {
                OutputFormat::Table => {
                    println!("Call ID: {}", call.id);
                    println!("  State: {}", call.state);
                    println!("  From: {}", format_party(&call.from_uri, &call.from_display_name));
                    println!("  To: {}", format_party(&call.to_uri, &call.to_display_name));
                    println!("  Created: {}", call.created_at);

                    if let Some(established) = &call.established_at {
                        println!("  Established: {}", established);
                    }

                    if let Some(ended) = &call.ended_at {
                        println!("  Ended: {}", ended);
                    }

                    if let Some(duration) = call.duration_seconds {
                        println!("  Duration: {}", format_duration(duration));
                    }

                    println!("  Media Anchored: {}", if call.media_anchored { "yes" } else { "no" });

                    if let Some(reason) = &call.failure_reason {
                        println!("  Failure Reason: {}", reason);
                    }
                }
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&call)?);
                }
                OutputFormat::Yaml => {
                    println!("{}", serde_yaml::to_string(&call)?);
                }
            }
        }

        CallCommands::Hangup { call_id, yes } => {
            // Confirm hangup unless --yes was provided
            if !yes {
                print!("Are you sure you want to hang up call '{}'? [y/N]: ", call_id);
                io::stdout().flush()?;

                let mut input = String::new();
                io::stdin().read_line(&mut input)?;

                if !input.trim().eq_ignore_ascii_case("y") {
                    println!("Hangup cancelled");
                    return Ok(());
                }
            }

            let response = client
                .hangup_call(&call_id)
                .await
                .context(format!("Failed to hang up call '{}'", call_id))?;

            match output_format {
                OutputFormat::Table => {
                    println!("Call '{}' hung up successfully (status: {})", response.call_id, response.status);
                }
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&response)?);
                }
                OutputFormat::Yaml => {
                    println!("{}", serde_yaml::to_string(&response)?);
                }
            }
        }

        CallCommands::Originate {
            from,
            to,
            from_display_name,
        } => {
            // This is a placeholder - call origination not yet implemented in server
            return Err(anyhow::anyhow!(
                "Call origination not yet implemented in fcp-server.\n\
                 Planned parameters: from={}, to={}, display_name={:?}",
                from, to, from_display_name
            ));
        }
    }

    Ok(())
}

/// Format a call party (URI + display name)
fn format_party(uri: &str, display_name: &Option<String>) -> String {
    match display_name {
        Some(name) if !name.is_empty() => format!("{} <{}>", name, uri),
        _ => uri.to_string(),
    }
}

/// Format duration in human-readable form (HH:MM:SS or MM:SS)
fn format_duration(seconds: u64) -> String {
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let secs = seconds % 60;

    if hours > 0 {
        format!("{:02}:{:02}:{:02}", hours, minutes, secs)
    } else {
        format!("{:02}:{:02}", minutes, secs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(0), "00:00");
        assert_eq!(format_duration(30), "00:30");
        assert_eq!(format_duration(90), "01:30");
        assert_eq!(format_duration(3661), "01:01:01");
        assert_eq!(format_duration(7200), "02:00:00");
    }

    #[test]
    fn test_format_party() {
        assert_eq!(
            format_party("sip:alice@example.com", &Some("Alice".to_string())),
            "Alice <sip:alice@example.com>"
        );
        assert_eq!(
            format_party("sip:bob@example.com", &None),
            "sip:bob@example.com"
        );
        assert_eq!(
            format_party("sip:charlie@example.com", &Some("".to_string())),
            "sip:charlie@example.com"
        );
    }
}
