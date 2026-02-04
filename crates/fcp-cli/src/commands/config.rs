//! Configuration command handlers

use crate::config::AppSettings;
use crate::{ConfigCommands, OutputFormat};
use anyhow::Result;

/// Handle config commands
pub async fn handle(cmd: ConfigCommands, output_format: OutputFormat) -> Result<()> {
    match cmd {
        ConfigCommands::Show => {
            let config = crate::config::Config::load_or_create()?;
            let settings = &config.settings;

            match output_format {
                OutputFormat::Json => {
                    let json = serde_json::to_string_pretty(settings)?;
                    println!("{}", json);
                }
                OutputFormat::Yaml => {
                    // Serialize via JSON value for yaml-like output
                    let value = serde_json::to_value(settings)?;
                    print_yaml_value(&value, 0);
                }
                OutputFormat::Table => {
                    println!("FCP Configuration");
                    println!("{}", "=".repeat(50));
                    println!();
                    for key in AppSettings::keys() {
                        if let Some(value) = settings.get(key) {
                            println!("  {:<30} {}", key, value);
                        }
                    }
                    println!();
                    println!("Config file: {}", config_file_display());
                }
            }
        }

        ConfigCommands::Set { key, value } => {
            let mut config = crate::config::Config::load_or_create()?;
            config.settings.set(&key, &value)?;
            config.save()?;

            println!("{} = {}", key, config.settings.get(&key).unwrap_or(value));
        }

        ConfigCommands::Get { key } => {
            let config = crate::config::Config::load_or_create()?;

            match config.settings.get(&key) {
                Some(value) => println!("{}", value),
                None => {
                    anyhow::bail!(
                        "Unknown configuration key '{}'. Valid keys:\n  {}",
                        key,
                        AppSettings::keys().join(", ")
                    );
                }
            }
        }
    }

    Ok(())
}

/// Display config file path (best-effort)
fn config_file_display() -> String {
    directories::ProjectDirs::from("com", "fcp", "cli")
        .map(|p| p.config_dir().join("config.toml").display().to_string())
        .unwrap_or_else(|| "~/.config/fcp/cli/config.toml".to_string())
}

/// Simple YAML-like printer for serde_json::Value
fn print_yaml_value(value: &serde_json::Value, indent: usize) {
    let prefix = " ".repeat(indent);
    match value {
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                match v {
                    serde_json::Value::Object(_) => {
                        println!("{}{}:", prefix, k);
                        print_yaml_value(v, indent + 2);
                    }
                    _ => {
                        println!("{}{}: {}", prefix, k, format_yaml_scalar(v));
                    }
                }
            }
        }
        _ => println!("{}{}", prefix, format_yaml_scalar(value)),
    }
}

fn format_yaml_scalar(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Null => "null".to_string(),
        other => other.to_string(),
    }
}
