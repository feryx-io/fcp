//! Context command handlers

use crate::config::{Config, Context};
use crate::OutputFormat;
use anyhow::Result;

pub async fn handle(cmd: crate::ContextCommands, output_format: OutputFormat) -> Result<()> {
    let mut config = Config::load_or_create()?;

    match cmd {
        crate::ContextCommands::List => match output_format {
            OutputFormat::Json => crate::output::print_json(&config.contexts),
            OutputFormat::Yaml => crate::output::print_yaml(&config.contexts),
            OutputFormat::Table => {
                println!("Available contexts:");
                for ctx in &config.contexts {
                    let current = if ctx.name == config.current_context {
                        " *"
                    } else {
                        "  "
                    };
                    println!("{}  {} - {}", current, ctx.name, ctx.url);
                }
            }
        },
        crate::ContextCommands::Current => {
            if let Some(ctx) = config.current() {
                match output_format {
                    OutputFormat::Json => crate::output::print_json(ctx),
                    OutputFormat::Yaml => crate::output::print_yaml(ctx),
                    OutputFormat::Table => {
                        println!("Current context: {}", ctx.name);
                        println!("  URL: {}", ctx.url);
                        if let Some(ref events_url) = ctx.events_url {
                            println!("  Events URL: {}", events_url);
                        }
                        if ctx.token.is_some() {
                            println!("  Token: configured");
                        }
                    }
                }
            } else {
                println!("No current context");
            }
        }
        crate::ContextCommands::Add { name, url, token, events_url } => {
            config.add_context(Context {
                name: name.clone(),
                url,
                token,
                events_url,
            })?;
            println!("Context '{}' added successfully", name);
        }
        crate::ContextCommands::Use { name } => {
            config.use_context(&name)?;
            println!("Switched to context '{}'", name);
        }
        crate::ContextCommands::Delete { name, yes } => {
            if !yes {
                print!("Delete context '{}'? (y/N): ", name);
                use std::io::{self, BufRead};
                let stdin = io::stdin();
                let mut line = String::new();
                stdin.lock().read_line(&mut line)?;
                if !line.trim().eq_ignore_ascii_case("y") {
                    println!("Cancelled");
                    return Ok(());
                }
            }
            config.delete_context(&name)?;
            println!("Context '{}' deleted", name);
        }
        crate::ContextCommands::Show { name } => {
            let ctx = if let Some(n) = name {
                config.get_context(&n)
            } else {
                config.current()
            };

            if let Some(ctx) = ctx {
                match output_format {
                    OutputFormat::Json => crate::output::print_json(ctx),
                    OutputFormat::Yaml => crate::output::print_yaml(ctx),
                    OutputFormat::Table => {
                        println!("Context: {}", ctx.name);
                        println!("  URL: {}", ctx.url);
                        if let Some(ref events_url) = ctx.events_url {
                            println!("  Events URL: {}", events_url);
                        }
                        if ctx.token.is_some() {
                            println!("  Token: configured");
                        }
                    }
                }
            } else {
                anyhow::bail!("Context not found");
            }
        }
    }

    Ok(())
}
