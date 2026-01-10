//! Output formatters for different formats (table, json, yaml)

use serde::Serialize;

pub fn print_table<T: Serialize>(data: &T) {
    // For generic table output, use JSON for now
    // Each command handler should implement custom table formatting
    println!("{}", serde_json::to_string_pretty(data).unwrap());
}

pub fn print_json<T: Serialize>(data: &T) {
    println!("{}", serde_json::to_string_pretty(data).unwrap());
}

pub fn print_yaml<T: Serialize>(data: &T) {
    match serde_yaml::to_string(data) {
        Ok(yaml) => println!("{}", yaml),
        Err(e) => eprintln!("Failed to serialize to YAML: {}", e),
    }
}
