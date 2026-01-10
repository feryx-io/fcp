//! Debug command handlers

use crate::api::ApiClient;
use crate::config::ResolvedContext;
use crate::{DebugCommands, OutputFormat};
use anyhow::{Context as _, Result};
use comfy_table::{presets::UTF8_FULL, ContentArrangement, Table};

/// Handle debug commands
pub async fn handle(
    cmd: DebugCommands,
    context: ResolvedContext,
    output_format: OutputFormat,
) -> Result<()> {
    let client = ApiClient::new(context.url, context.token)?;

    match cmd {
        DebugCommands::Summary => {
            let summary = client
                .get_debug_summary()
                .await
                .context("Failed to get debug summary")?;

            match output_format {
                OutputFormat::Table => {
                    println!("Debug Summary:");
                    println!("  Active Dialogs: {}", summary.active_dialogs);
                    println!("  Active Subscriptions: {}", summary.active_subscriptions);
                    println!("  Active Calls: {}", summary.active_calls);
                    println!("  Rate Limit Entries: {}", summary.rate_limit_entries);
                }
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&summary)?);
                }
                OutputFormat::Yaml => {
                    println!("{}", serde_yaml::to_string(&summary)?);
                }
            }
        }

        DebugCommands::Dialogs => {
            let dialogs = client
                .list_dialogs()
                .await
                .context("Failed to list dialogs")?;

            match output_format {
                OutputFormat::Table => {
                    if dialogs.is_empty() {
                        println!("No active dialogs");
                        return Ok(());
                    }

                    let mut table = Table::new();
                    table
                        .load_preset(UTF8_FULL)
                        .set_content_arrangement(ContentArrangement::Dynamic)
                        .set_header(vec![
                            "Call ID",
                            "Local Tag",
                            "Remote Tag",
                            "State",
                            "Local CSeq",
                            "Remote CSeq",
                        ]);

                    for dialog in &dialogs {
                        table.add_row(vec![
                            dialog.call_id.clone(),
                            dialog.local_tag.clone(),
                            dialog.remote_tag.clone(),
                            dialog.state.clone(),
                            dialog.local_cseq.to_string(),
                            dialog.remote_cseq.to_string(),
                        ]);
                    }

                    println!("{table}");
                    println!("\nTotal active dialogs: {}", dialogs.len());
                }
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&dialogs)?);
                }
                OutputFormat::Yaml => {
                    println!("{}", serde_yaml::to_string(&dialogs)?);
                }
            }
        }

        DebugCommands::Subscriptions => {
            let subscriptions = client
                .list_subscriptions()
                .await
                .context("Failed to list subscriptions")?;

            match output_format {
                OutputFormat::Table => {
                    if subscriptions.is_empty() {
                        println!("No active subscriptions");
                        return Ok(());
                    }

                    let mut table = Table::new();
                    table
                        .load_preset(UTF8_FULL)
                        .set_content_arrangement(ContentArrangement::Dynamic)
                        .set_header(vec![
                            "Call ID",
                            "From Tag",
                            "To Tag",
                            "Event",
                            "State",
                            "Expires At",
                        ]);

                    for subscription in &subscriptions {
                        table.add_row(vec![
                            subscription.call_id.clone(),
                            subscription.from_tag.clone(),
                            subscription.to_tag.clone(),
                            subscription.event.clone(),
                            subscription.state.clone(),
                            subscription.expires_at.clone(),
                        ]);
                    }

                    println!("{table}");
                    println!("\nTotal active subscriptions: {}", subscriptions.len());
                }
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&subscriptions)?);
                }
                OutputFormat::Yaml => {
                    println!("{}", serde_yaml::to_string(&subscriptions)?);
                }
            }
        }

        DebugCommands::RateLimits => {
            let rate_limits = client
                .list_rate_limits()
                .await
                .context("Failed to list rate limits")?;

            match output_format {
                OutputFormat::Table => {
                    if rate_limits.is_empty() {
                        println!("No rate limit entries");
                        return Ok(());
                    }

                    let mut table = Table::new();
                    table
                        .load_preset(UTF8_FULL)
                        .set_content_arrangement(ContentArrangement::Dynamic)
                        .set_header(vec![
                            "Dialog ID",
                            "Window Start",
                            "Request Counts",
                        ]);

                    for limit in &rate_limits {
                        // Format the counts map as a readable string
                        let counts_str = format_counts(&limit.counts);

                        table.add_row(vec![
                            limit.dialog_id.clone(),
                            limit.window_start.clone(),
                            counts_str,
                        ]);
                    }

                    println!("{table}");
                    println!("\nTotal rate limit entries: {}", rate_limits.len());
                }
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&rate_limits)?);
                }
                OutputFormat::Yaml => {
                    println!("{}", serde_yaml::to_string(&rate_limits)?);
                }
            }
        }

        DebugCommands::Calls => {
            let calls = client
                .list_debug_calls()
                .await
                .context("Failed to list debug calls")?;

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
                            "Media Session ID",
                            "A-Leg Dialog ID",
                            "B-Leg Dialog ID",
                        ]);

                    for call in &calls {
                        table.add_row(vec![
                            call.call_id.clone(),
                            call.state.clone(),
                            call.media_session_id.clone().unwrap_or_else(|| "-".to_string()),
                            call.a_leg_dialog_id.clone().unwrap_or_else(|| "-".to_string()),
                            call.b_leg_dialog_id.clone().unwrap_or_else(|| "-".to_string()),
                        ]);
                    }

                    println!("{table}");
                    println!("\nTotal active calls (debug view): {}", calls.len());
                }
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&calls)?);
                }
                OutputFormat::Yaml => {
                    println!("{}", serde_yaml::to_string(&calls)?);
                }
            }
        }
    }

    Ok(())
}

/// Format a counts HashMap as a readable string
fn format_counts(counts: &std::collections::HashMap<String, u32>) -> String {
    if counts.is_empty() {
        return "-".to_string();
    }

    let mut pairs: Vec<_> = counts.iter().collect();
    pairs.sort_by_key(|(k, _)| *k);

    pairs
        .iter()
        .map(|(method, count)| format!("{}:{}", method, count))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_format_counts_empty() {
        let counts = HashMap::new();
        assert_eq!(format_counts(&counts), "-");
    }

    #[test]
    fn test_format_counts_single() {
        let mut counts = HashMap::new();
        counts.insert("INVITE".to_string(), 5);
        assert_eq!(format_counts(&counts), "INVITE:5");
    }

    #[test]
    fn test_format_counts_multiple() {
        let mut counts = HashMap::new();
        counts.insert("INVITE".to_string(), 5);
        counts.insert("BYE".to_string(), 3);
        counts.insert("ACK".to_string(), 10);
        // Should be sorted alphabetically by method
        let result = format_counts(&counts);
        assert!(result.contains("ACK:10"));
        assert!(result.contains("BYE:3"));
        assert!(result.contains("INVITE:5"));
    }
}
