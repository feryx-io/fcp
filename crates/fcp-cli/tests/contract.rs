//! The CLI reads what the API's contract says the API answers
//! (`docs/DEVELOPER_SURFACE.md` §3, phase 3b).
//!
//! Each model the CLI decodes an admin API response into is fed instances
//! built from that response's schema in `docs/openapi.json`: a minimal one
//! (only the required properties, `null` wherever null is allowed) and a
//! full one (every property, never `null`). A model that insists on a field
//! the API may leave out, or expects a type the API does not send, fails
//! here instead of at a user's terminal.

use fcp_cli::api::*;
use serde::de::DeserializeOwned;
use serde_json::{json, Map, Value};

fn spec() -> Value {
    serde_json::from_str(include_str!("../../../docs/openapi.json")).expect("docs/openapi.json")
}

/// Which way to build an instance.
#[derive(Clone, Copy, PartialEq)]
enum Fill {
    /// Required properties only; `null` where allowed.
    Minimal,
    /// Every property, and a value rather than `null`.
    Full,
}

fn resolve<'a>(spec: &'a Value, schema: &'a Value) -> &'a Value {
    match schema.get("$ref").and_then(Value::as_str) {
        Some(r) => {
            let name = r.trim_start_matches("#/components/schemas/");
            resolve(spec, &spec["components"]["schemas"][name])
        }
        None => schema,
    }
}

fn types_of(schema: &Value) -> Vec<String> {
    match &schema["type"] {
        Value::String(t) => vec![t.clone()],
        Value::Array(ts) => ts
            .iter()
            .filter_map(|t| t.as_str().map(String::from))
            .collect(),
        _ => vec![],
    }
}

fn instance(spec: &Value, schema: &Value, fill: Fill, depth: usize) -> Value {
    let schema = resolve(spec, schema);
    assert!(depth < 12, "the schema nests too deep: {schema}");
    for key in ["oneOf", "anyOf"] {
        if let Some(options) = schema[key].as_array() {
            let non_null: Vec<&Value> = options
                .iter()
                .filter(|o| types_of(resolve(spec, o)) != ["null"])
                .collect();
            let has_null = non_null.len() < options.len();
            if fill == Fill::Minimal && has_null {
                return Value::Null;
            }
            return instance(
                spec,
                non_null.first().copied().unwrap_or(&options[0]),
                fill,
                depth + 1,
            );
        }
    }
    if let Some(all) = schema["allOf"].as_array() {
        let mut merged = Map::new();
        for part in all {
            if let Value::Object(m) = instance(spec, part, fill, depth + 1) {
                merged.extend(m);
            }
        }
        return Value::Object(merged);
    }
    if let Some(values) = schema["enum"].as_array() {
        return values
            .iter()
            .find(|v| !v.is_null())
            .cloned()
            .unwrap_or(Value::Null);
    }
    let types = types_of(schema);
    if fill == Fill::Minimal && types.iter().any(|t| t == "null") {
        return Value::Null;
    }
    let ty = types
        .iter()
        .find(|t| *t != "null")
        .cloned()
        .unwrap_or_default();
    match ty.as_str() {
        "string" => match schema["format"].as_str() {
            Some("date-time") => json!("2026-09-29T12:00:00Z"),
            _ => json!("x"),
        },
        "integer" => json!(1),
        "number" => json!(1.5),
        "boolean" => json!(true),
        "array" => match fill {
            Fill::Minimal => json!([]),
            Fill::Full => json!([instance(spec, &schema["items"], fill, depth + 1)]),
        },
        _ => {
            let required: Vec<&str> = schema["required"]
                .as_array()
                .map(|r| r.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            let mut out = Map::new();
            if let Some(props) = schema["properties"].as_object() {
                for (name, prop) in props {
                    if fill == Fill::Full || required.contains(&name.as_str()) {
                        out.insert(name.clone(), instance(spec, prop, fill, depth + 1));
                    }
                }
            }
            Value::Object(out)
        }
    }
}

fn reads<T: DeserializeOwned>(model: &str, schema: &str) {
    let spec = spec();
    let root = &spec["components"]["schemas"][schema];
    assert!(root.is_object(), "{schema} is not in the contract");
    for fill in [Fill::Minimal, Fill::Full] {
        let value = instance(
            &spec,
            &json!({ "$ref": format!("#/components/schemas/{schema}") }),
            fill,
            0,
        );
        let kind = if fill == Fill::Minimal {
            "minimal"
        } else {
            "full"
        };
        if let Err(e) = serde_json::from_value::<T>(value.clone()) {
            panic!("the CLI's {model} cannot read a {kind} {schema}: {e}\n{value:#}");
        }
    }
}

#[test]
fn the_cli_reads_what_the_api_answers() {
    reads::<TenantResponse>("TenantResponse", "TenantResponse");
    reads::<UserResponse>("UserResponse", "UserResponse");
    reads::<RegistrationResponse>("RegistrationResponse", "RegistrationResponse");
    reads::<CallResponse>("CallResponse", "CallInfo");
    reads::<PhoneNumberResponse>("PhoneNumberResponse", "PhoneNumberResponse");
    reads::<DeviceResponse>("DeviceResponse", "DeviceResponse");
    reads::<LineBindingResponse>("LineBindingResponse", "LineBindingResponse");
    reads::<DialogResponse>("DialogResponse", "DialogInfo");
    reads::<SubscriptionResponse>("SubscriptionResponse", "SubscriptionInfo");
    reads::<RateLimitResponse>("RateLimitResponse", "RateLimitInfo");
    reads::<DebugCallResponse>("DebugCallResponse", "debug.CallInfo");
    reads::<DebugSummaryResponse>("DebugSummaryResponse", "DebugSummary");
    reads::<StatusResponse>("StatusResponse", "StatusResponse");
}
