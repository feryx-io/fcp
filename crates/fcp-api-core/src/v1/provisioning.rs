//! Device auto-provisioning endpoint and template engine.

use crate::{error::ApiError, state::ApiState};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use fcp_core::storage::DeviceStore;
use std::collections::HashMap;

/// Simple template engine -- replaces ${variable} placeholders.
fn render_template(template_body: &str, vars: &HashMap<String, String>) -> String {
    let mut result = template_body.to_string();
    for (key, value) in vars {
        result = result.replace(&format!("${{{}}}", key), value);
    }
    result
}

/// GET /provisioning/:mac_address -- Auto-provision a phone by MAC address.
///
/// No authentication required -- phones request their config at boot using
/// their MAC address as the sole identifying factor.
pub async fn auto_provision(
    State(state): State<ApiState>,
    Path(mac_address): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::Internal("Storage not configured".into()))?;

    // Normalize MAC: strip colons/dashes/dots, lowercase
    let normalized_mac = mac_address.replace([':', '-', '.'], "").to_lowercase();

    // Look up device by MAC
    let device = storage
        .get_device_by_mac_address(&normalized_mac)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::NotFound("Device not found".into()))?;

    if !device.enabled {
        return Err(ApiError::Forbidden("Device is disabled".into()));
    }

    // Find the config template
    let template_id = device
        .config_template_id
        .as_ref()
        .ok_or_else(|| ApiError::NotFound("No config template assigned to device".into()))?;

    let template_id = template_id
        .parse::<fcp_core::ConfigTemplateId>()
        .map_err(|e| ApiError::BadRequest(format!("Invalid template ID: {}", e)))?;

    let template_store = storage
        .config_template_store()
        .ok_or_else(|| ApiError::Internal("Config template store not configured".into()))?;

    let template = template_store
        .get_config_template(&template_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::NotFound("Config template not found".into()))?;
    if template.tenant_id != device.tenant_id {
        return Err(ApiError::NotFound("Config template not found".into()));
    }

    // Build variable map
    let mut vars = HashMap::new();
    vars.insert("mac_address".to_string(), normalized_mac);
    vars.insert("device_name".to_string(), device.name.clone());

    // Get owner's SIP credentials if device has an owner
    if let Some(ref owner_id) = device.owner_user_id {
        if let Some(ref sip_uri) = device.sip_uri {
            vars.insert("sip_uri".to_string(), sip_uri.clone());
            // Extract username from sip:user@host
            if let Some(user) = sip_uri
                .strip_prefix("sip:")
                .and_then(|s| s.split('@').next())
            {
                vars.insert("sip_username".to_string(), user.to_string());
            }
        }
        vars.insert("owner_user_id".to_string(), owner_id.to_string());
    }

    // Add SIP server info
    vars.insert("sip_server".to_string(), state.realm.clone());
    vars.insert("sip_port".to_string(), "5060".to_string());

    if let Some(ref model) = device.model {
        vars.insert("model".to_string(), model.clone());
    }

    // Render template
    let config = render_template(&template.template_body, &vars);

    // Return as text (could be XML, INI, etc. depending on vendor)
    Ok((
        StatusCode::OK,
        [("content-type", "text/plain; charset=utf-8")],
        config,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_template() {
        let template = "server=${sip_server}\nuser=${sip_username}\nmac=${mac_address}";
        let mut vars = HashMap::new();
        vars.insert("sip_server".to_string(), "pbx.example.com".to_string());
        vars.insert("sip_username".to_string(), "alice".to_string());
        vars.insert("mac_address".to_string(), "001122334455".to_string());

        let result = render_template(template, &vars);
        assert_eq!(
            result,
            "server=pbx.example.com\nuser=alice\nmac=001122334455"
        );
    }

    #[test]
    fn test_render_template_missing_vars_left_as_is() {
        let template = "server=${sip_server}\npassword=${password}";
        let mut vars = HashMap::new();
        vars.insert("sip_server".to_string(), "pbx.example.com".to_string());

        let result = render_template(template, &vars);
        assert_eq!(result, "server=pbx.example.com\npassword=${password}");
    }
}
