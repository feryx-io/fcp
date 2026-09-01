//! Device auto-provisioning endpoint and template engine.

use crate::{error::ApiError, state::ApiState};
use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    response::IntoResponse,
};
use fcp_core::{storage::DeviceStore, CredentialStore, Device};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

const PROVISIONING_NOT_FOUND: &str = "Provisioning config not found";
const PROVISIONING_TOKEN_HEADER: &str = "x-fcp-provisioning-token";
const DEVICE_PROVISIONING_TOKEN_HEADER: &str = "x-fcp-device-provisioning-token";

/// Simple template engine -- replaces ${variable} placeholders.
fn render_template(template_body: &str, vars: &HashMap<String, String>) -> String {
    let mut result = template_body.to_string();
    for (key, value) in vars {
        result = result.replace(&format!("${{{}}}", key), value);
    }
    result
}

fn provisioning_not_found() -> ApiError {
    ApiError::NotFound(PROVISIONING_NOT_FOUND.to_string())
}

/// Returns `true` when the request arrived over HTTPS, as indicated by the
/// `X-Forwarded-Proto` header set by a reverse proxy.  When no header is
/// present we assume plain HTTP (fail-closed).
fn request_is_https(headers: &HeaderMap) -> bool {
    headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .is_some_and(|proto| proto.eq_ignore_ascii_case("https"))
}

fn provisioning_token_from_headers(headers: &HeaderMap) -> Option<String> {
    headers
        .get(PROVISIONING_TOKEN_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            headers
                .get(header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.strip_prefix("Bearer "))
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
}

fn device_provisioning_token_from_headers(headers: &HeaderMap) -> Option<String> {
    headers
        .get(DEVICE_PROVISIONING_TOKEN_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn constant_time_compare(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }

    let mut diff = 0u8;
    for (left, right) in left.as_bytes().iter().zip(right.as_bytes()) {
        diff |= left ^ right;
    }
    diff == 0
}

async fn authorize_shared_provisioning_token(
    expected_token: Option<&str>,
    headers: &HeaderMap,
) -> Result<(), ApiError> {
    let expected_token = expected_token.ok_or_else(provisioning_not_found)?;
    let presented_token =
        provisioning_token_from_headers(headers).ok_or_else(provisioning_not_found)?;

    let auth_config = crate::auth::AuthConfig::new(expected_token.to_string());
    if crate::auth::validate_token(&presented_token, &auth_config)
        .await
        .is_none()
    {
        return Err(provisioning_not_found());
    }

    Ok(())
}

fn authorize_device_provisioning_token(
    device: &Device,
    headers: &HeaderMap,
) -> Result<(), ApiError> {
    let expected_device_token_hash = device
        .provisioning_token_hash
        .as_deref()
        .filter(|hash| !hash.trim().is_empty())
        .ok_or_else(provisioning_not_found)?;
    let presented_device_token =
        device_provisioning_token_from_headers(headers).ok_or_else(provisioning_not_found)?;
    let presented_device_token_hash = hash_token(&presented_device_token);
    if !constant_time_compare(&presented_device_token_hash, expected_device_token_hash) {
        return Err(provisioning_not_found());
    }

    Ok(())
}

/// GET /provisioning/:mac_address -- Auto-provision a phone by MAC address.
///
/// Phones must present both:
/// - `Authorization: Bearer <token>`
/// - or `X-FCP-Provisioning-Token: <token>`
/// - and `X-FCP-Device-Provisioning-Token: <per-device-token>`
///
/// Unknown or unauthorized devices all return the same 404 to avoid turning
/// this endpoint into an enumeration surface.
pub async fn auto_provision(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(mac_address): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    if state.provisioning_require_https && !request_is_https(&headers) {
        return Err(provisioning_not_found());
    }

    // Gate the storage lookup on the shared token so unauthenticated callers
    // can't probe MAC existence by timing or spin the DB.
    authorize_shared_provisioning_token(state.provisioning_auth_token.as_deref(), &headers).await?;

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
        .ok_or_else(provisioning_not_found)?;

    // Verify the per-device token before inspecting device state so a caller
    // with only the shared token cannot distinguish "disabled" from "enabled
    // but wrong per-device token" by response timing.
    authorize_device_provisioning_token(&device, &headers)?;

    if !device.enabled {
        return Err(provisioning_not_found());
    }

    // Find the config template
    let template_id = device
        .config_template_id
        .as_ref()
        .ok_or_else(provisioning_not_found)?;

    let template_id = template_id
        .parse::<fcp_core::ConfigTemplateId>()
        .map_err(|_| provisioning_not_found())?;

    let template_store = storage
        .config_template_store()
        .ok_or_else(|| ApiError::Internal("Config template store not configured".into()))?;

    let template = template_store
        .get_config_template(&template_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(provisioning_not_found)?;
    if template.tenant_id != device.tenant_id {
        return Err(provisioning_not_found());
    }

    // Build variable map
    let mut vars = HashMap::new();
    vars.insert("mac_address".to_string(), normalized_mac);
    vars.insert("device_name".to_string(), device.name.clone());

    // Get owner's SIP credentials if device has an owner
    if let Some(ref owner_id) = device.owner_user_id {
        let owner = storage
            .get_user_by_id(owner_id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
            .filter(|owner| owner.tenant_id == device.tenant_id && owner.enabled)
            .ok_or_else(provisioning_not_found)?;

        let sip_ha1_hash = owner
            .sip_ha1_hash
            .clone()
            .ok_or_else(provisioning_not_found)?;
        let sip_uri = device
            .sip_uri
            .clone()
            .unwrap_or_else(|| format!("sip:{}@{}", owner.username, state.realm));

        vars.insert("sip_uri".to_string(), sip_uri);
        vars.insert("sip_username".to_string(), owner.username.clone());
        vars.insert("sip_auth_username".to_string(), owner.username.clone());
        vars.insert("sip_auth_ha1_hash".to_string(), sip_ha1_hash);
        if let Some(ref sip_password) = owner.sip_provisioning_password {
            vars.insert("sip_password".to_string(), sip_password.clone());
            vars.insert(
                "sip_provisioning_password".to_string(),
                sip_password.clone(),
            );
        }
        vars.insert("owner_username".to_string(), owner.username.clone());
        if let Some(ref display_name) = owner.display_name {
            vars.insert("owner_display_name".to_string(), display_name.clone());
        }
        if let Some(ref email) = owner.email {
            vars.insert("owner_email".to_string(), email.clone());
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
        [
            ("content-type", "text/plain; charset=utf-8"),
            ("cache-control", "no-store, no-cache, max-age=0"),
            ("pragma", "no-cache"),
            ("expires", "0"),
            ("x-content-type-options", "nosniff"),
        ],
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

    #[test]
    fn test_request_is_https() {
        let mut headers = HeaderMap::new();
        assert!(!request_is_https(&headers), "no header → plain HTTP");

        headers.insert("x-forwarded-proto", "http".parse().unwrap());
        assert!(!request_is_https(&headers));

        headers.insert("x-forwarded-proto", "https".parse().unwrap());
        assert!(request_is_https(&headers));

        headers.insert("x-forwarded-proto", "HTTPS".parse().unwrap());
        assert!(request_is_https(&headers), "case-insensitive");
    }

    #[test]
    fn test_provisioning_token_from_custom_header() {
        let mut headers = HeaderMap::new();
        headers.insert(
            PROVISIONING_TOKEN_HEADER,
            "shared-token".parse().expect("valid header"),
        );

        assert_eq!(
            provisioning_token_from_headers(&headers).as_deref(),
            Some("shared-token")
        );
    }

    #[test]
    fn test_provisioning_token_from_bearer_header() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            "Bearer shared-token".parse().expect("valid header"),
        );

        assert_eq!(
            provisioning_token_from_headers(&headers).as_deref(),
            Some("shared-token")
        );
    }

    #[test]
    fn test_device_provisioning_token_from_header() {
        let mut headers = HeaderMap::new();
        headers.insert(
            DEVICE_PROVISIONING_TOKEN_HEADER,
            " device-token ".parse().expect("valid header"),
        );

        assert_eq!(
            device_provisioning_token_from_headers(&headers).as_deref(),
            Some("device-token")
        );
    }

    fn test_device_with_token_hash(hash: Option<&str>) -> Device {
        let mut device = Device::new(
            "Test Phone".to_string(),
            fcp_core::DeviceType::DeskPhone,
            fcp_core::TenantId::new(),
        );
        device.provisioning_token_hash = hash.map(str::to_string);
        device
    }

    #[test]
    fn test_authorize_device_provisioning_rejects_when_hash_unset() {
        let device = test_device_with_token_hash(None);
        let mut headers = HeaderMap::new();
        headers.insert(
            DEVICE_PROVISIONING_TOKEN_HEADER,
            "anything".parse().unwrap(),
        );

        let err = authorize_device_provisioning_token(&device, &headers)
            .expect_err("device without a hash must fail closed");
        assert!(matches!(err, ApiError::NotFound(_)));
    }

    #[test]
    fn test_authorize_device_provisioning_rejects_when_header_missing() {
        let hash = hash_token("correct-horse-battery-staple-correct-horse");
        let device = test_device_with_token_hash(Some(&hash));
        let headers = HeaderMap::new();

        let err = authorize_device_provisioning_token(&device, &headers)
            .expect_err("missing header must 404");
        assert!(matches!(err, ApiError::NotFound(_)));
    }

    #[test]
    fn test_authorize_device_provisioning_rejects_wrong_token() {
        let hash = hash_token("correct-horse-battery-staple-correct-horse");
        let device = test_device_with_token_hash(Some(&hash));
        let mut headers = HeaderMap::new();
        headers.insert(
            DEVICE_PROVISIONING_TOKEN_HEADER,
            "wrong-token-but-also-32-chars-long-xx".parse().unwrap(),
        );

        let err = authorize_device_provisioning_token(&device, &headers)
            .expect_err("wrong token must 404");
        assert!(matches!(err, ApiError::NotFound(_)));
    }

    #[test]
    fn test_authorize_device_provisioning_accepts_correct_token() {
        let token = "correct-horse-battery-staple-correct-horse";
        let hash = hash_token(token);
        let device = test_device_with_token_hash(Some(&hash));
        let mut headers = HeaderMap::new();
        headers.insert(DEVICE_PROVISIONING_TOKEN_HEADER, token.parse().unwrap());

        authorize_device_provisioning_token(&device, &headers)
            .expect("correct token must authorize");
    }

    #[tokio::test]
    async fn test_authorize_shared_provisioning_rejects_when_unconfigured() {
        let mut headers = HeaderMap::new();
        headers.insert(PROVISIONING_TOKEN_HEADER, "any-token".parse().unwrap());

        let err = authorize_shared_provisioning_token(None, &headers)
            .await
            .expect_err("unset deployment token must fail closed");
        assert!(matches!(err, ApiError::NotFound(_)));
    }

    #[tokio::test]
    async fn test_authorize_shared_provisioning_rejects_when_header_missing() {
        let headers = HeaderMap::new();
        let err = authorize_shared_provisioning_token(Some("shared-token"), &headers)
            .await
            .expect_err("missing header must 404");
        assert!(matches!(err, ApiError::NotFound(_)));
    }

    #[tokio::test]
    async fn test_authorize_shared_provisioning_rejects_wrong_token() {
        let mut headers = HeaderMap::new();
        headers.insert(PROVISIONING_TOKEN_HEADER, "wrong-token".parse().unwrap());
        let err = authorize_shared_provisioning_token(Some("shared-token"), &headers)
            .await
            .expect_err("wrong token must 404");
        assert!(matches!(err, ApiError::NotFound(_)));
    }

    #[tokio::test]
    async fn test_authorize_shared_provisioning_accepts_correct_token() {
        let mut headers = HeaderMap::new();
        headers.insert(PROVISIONING_TOKEN_HEADER, "shared-token".parse().unwrap());
        authorize_shared_provisioning_token(Some("shared-token"), &headers)
            .await
            .expect("correct token must authorize");
    }

    #[test]
    fn test_constant_time_compare_basics() {
        assert!(constant_time_compare("abc", "abc"));
        assert!(!constant_time_compare("abc", "abd"));
        assert!(
            !constant_time_compare("abc", "abcd"),
            "length mismatch must fail"
        );
        assert!(constant_time_compare("", ""));
    }
}
