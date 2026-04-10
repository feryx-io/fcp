//! XCAP server (RFC 4825) for managing resource-lists and pres-rules documents.
//!
//! Provides whole-document GET/PUT/DELETE for:
//! - `resource-lists` (RFC 4826)
//! - `pres-rules` (RFC 5025)
//! - `xcap-caps` capability discovery

use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use fcp_core::storage::StoredResourceList;
use fcp_presence::xcap::{
    generate_pres_rules, generate_resource_lists_xml, parse_pres_rules, parse_resource_lists_xml,
    ResourceListEntryXml, ResourceListXml,
};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tracing::warn;

use crate::http_api::AppState;

// ============================================================================
// ETag helpers
// ============================================================================

/// Compute an ETag from document content: first 16 hex chars of SHA-256.
fn compute_etag(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let hash = hasher.finalize();
    let hex: String = hash.iter().take(8).map(|b| format!("{b:02x}")).collect();
    format!("\"{hex}\"")
}

/// Check `If-Match` header. Returns Err(412) if it doesn't match.
fn check_if_match(headers: &HeaderMap, current_etag: &str) -> Result<(), Box<Response>> {
    if let Some(if_match) = headers.get(header::IF_MATCH) {
        let val = if_match.to_str().unwrap_or("");
        if val != "*" && val != current_etag {
            return Err(Box::new(
                (StatusCode::PRECONDITION_FAILED, "ETag mismatch").into_response(),
            ));
        }
    }
    Ok(())
}

/// Check `If-None-Match` header. Returns Some(304) if it matches (for GET).
fn check_if_none_match(headers: &HeaderMap, current_etag: &str) -> Option<Response> {
    if let Some(if_none_match) = headers.get(header::IF_NONE_MATCH) {
        let val = if_none_match.to_str().unwrap_or("");
        if val == current_etag {
            return Some(StatusCode::NOT_MODIFIED.into_response());
        }
    }
    None
}

// ============================================================================
// XCAP error type (RFC 4825 §11)
// ============================================================================

fn xcap_error_response(status: StatusCode, error_type: &str, phrase: Option<&str>) -> Response {
    let phrase_attr = phrase
        .map(|p| format!(" phrase=\"{p}\""))
        .unwrap_or_default();
    let body = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <xcap-error xmlns=\"urn:ietf:params:xml:ns:xcap-error\">\n\
         <{error_type}{phrase_attr}/>\n\
         </xcap-error>"
    );
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/xcap-error+xml")
        .body(Body::from(body))
        .unwrap()
}

// ============================================================================
// Router
// ============================================================================

/// Build the XCAP sub-router. Must be nested under the XCAP root path.
pub fn xcap_router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/xcap-caps/global/index", get(get_xcap_caps))
        .route(
            "/resource-lists/users/{xui}/index",
            get(get_resource_lists)
                .put(put_resource_lists)
                .delete(delete_resource_lists),
        )
        .route(
            "/pres-rules/users/{xui}/index",
            get(get_pres_rules)
                .put(put_pres_rules)
                .delete(delete_pres_rules),
        )
}

// ============================================================================
// xcap-caps
// ============================================================================

async fn get_xcap_caps() -> impl IntoResponse {
    let body = r#"<?xml version="1.0" encoding="UTF-8"?>
<xcap-caps xmlns="urn:ietf:params:xml:ns:xcap-caps">
  <auids>
    <auid>resource-lists</auid>
    <auid>pres-rules</auid>
  </auids>
  <namespaces>
    <namespace>urn:ietf:params:xml:ns:resource-lists</namespace>
    <namespace>urn:ietf:params:xml:ns:common-policy</namespace>
    <namespace>urn:ietf:params:xml:ns:pres-rules</namespace>
  </namespaces>
</xcap-caps>"#;
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/xcap-caps+xml")
        .body(Body::from(body))
        .unwrap()
}

// ============================================================================
// resource-lists handlers
// ============================================================================

/// Synthetic URI for XCAP resource-lists storage key.
fn resource_lists_uri(xui: &str) -> String {
    format!("urn:xcap:resource-lists:{xui}")
}

async fn get_resource_lists(
    State(state): State<Arc<AppState>>,
    Path(xui): Path<String>,
    headers: HeaderMap,
) -> Response {
    let uri = resource_lists_uri(&xui);
    let store = &state.resource_list_store;

    match store.get_resource_list(&state.default_tenant_id, &uri).await {
        Ok(Some(stored)) => {
            // Decode entries_json → Vec<ResourceListXml>
            let lists: Vec<ResourceListXml> = match decode_resource_lists(&stored) {
                Ok(l) => l,
                Err(e) => {
                    warn!(error = %e, "Failed to decode stored resource lists");
                    return xcap_error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "cannot-decode",
                        Some("Stored document is corrupt"),
                    );
                }
            };

            let xml = generate_resource_lists_xml(&lists);
            let etag = compute_etag(&xml);

            if let Some(not_modified) = check_if_none_match(&headers, &etag) {
                return not_modified;
            }

            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "application/resource-lists+xml")
                .header(header::ETAG, &etag)
                .body(Body::from(xml))
                .unwrap()
        }
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            warn!(error = %e, "Storage error fetching resource lists");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn put_resource_lists(
    State(state): State<Arc<AppState>>,
    Path(xui): Path<String>,
    headers: HeaderMap,
    body: String,
) -> Response {
    // Validate Content-Type
    if let Some(ct) = headers.get(header::CONTENT_TYPE) {
        let ct_str = ct.to_str().unwrap_or("");
        if !ct_str.contains("application/resource-lists+xml") && !ct_str.contains("application/xml")
        {
            return xcap_error_response(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "not-xml-content-type",
                Some("Expected application/resource-lists+xml"),
            );
        }
    }

    // Parse the incoming XML
    let lists = match parse_resource_lists_xml(&body) {
        Ok(l) => l,
        Err(e) => {
            return xcap_error_response(
                StatusCode::BAD_REQUEST,
                "not-well-formed",
                Some(&format!("Invalid resource-lists XML: {e}")),
            );
        }
    };

    let uri = resource_lists_uri(&xui);
    let store = &state.resource_list_store;

    // Check If-Match against existing document
    let existing = store
        .get_resource_list(&state.default_tenant_id, &uri)
        .await
        .ok()
        .flatten();
    let is_create = existing.is_none();

    if let Some(ref existing) = existing {
        let existing_lists = decode_resource_lists(existing).unwrap_or_default();
        let existing_xml = generate_resource_lists_xml(&existing_lists);
        let current_etag = compute_etag(&existing_xml);
        if let Err(resp) = check_if_match(&headers, &current_etag) {
            return *resp;
        }
    } else if headers.contains_key(header::IF_MATCH) {
        // If-Match on non-existent document → 412
        return (StatusCode::PRECONDITION_FAILED, "Document does not exist").into_response();
    }

    // Encode and store
    let entries_json = encode_resource_lists(&lists);
    let stored = StoredResourceList {
        uri,
        name: lists.first().and_then(|l| l.name.clone()),
        entries_json,
        tenant_id: state.default_tenant_id,
    };

    if let Err(e) = store.save_resource_list(&stored).await {
        warn!(error = %e, "Failed to save resource list");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    let response_xml = generate_resource_lists_xml(&lists);
    let etag = compute_etag(&response_xml);

    let status = if is_create {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };

    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/resource-lists+xml")
        .header(header::ETAG, &etag)
        .body(Body::from(response_xml))
        .unwrap()
}

async fn delete_resource_lists(
    State(state): State<Arc<AppState>>,
    Path(xui): Path<String>,
    headers: HeaderMap,
) -> Response {
    let uri = resource_lists_uri(&xui);
    let store = &state.resource_list_store;

    // Check document exists
    let existing = match store.get_resource_list(&state.default_tenant_id, &uri).await {
        Ok(Some(stored)) => stored,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            warn!(error = %e, "Storage error");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // Check If-Match
    let existing_lists = decode_resource_lists(&existing).unwrap_or_default();
    let existing_xml = generate_resource_lists_xml(&existing_lists);
    let current_etag = compute_etag(&existing_xml);
    if let Err(resp) = check_if_match(&headers, &current_etag) {
        return *resp;
    }

    if let Err(e) = store.delete_resource_list(&state.default_tenant_id, &uri).await {
        warn!(error = %e, "Failed to delete resource list");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    StatusCode::OK.into_response()
}

// ============================================================================
// pres-rules handlers
// ============================================================================

async fn get_pres_rules(
    State(state): State<Arc<AppState>>,
    Path(xui): Path<String>,
    headers: HeaderMap,
) -> Response {
    let policy = state
        .manager
        .get_auth_policy_for_tenant(&xui, &state.default_tenant_id)
        .unwrap_or_default();
    let xml = generate_pres_rules(&policy);
    let etag = compute_etag(&xml);

    if let Some(not_modified) = check_if_none_match(&headers, &etag) {
        return not_modified;
    }

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/auth-policy+xml")
        .header(header::ETAG, &etag)
        .body(Body::from(xml))
        .unwrap()
}

async fn put_pres_rules(
    State(state): State<Arc<AppState>>,
    Path(xui): Path<String>,
    headers: HeaderMap,
    body: String,
) -> Response {
    // Validate Content-Type
    if let Some(ct) = headers.get(header::CONTENT_TYPE) {
        let ct_str = ct.to_str().unwrap_or("");
        if !ct_str.contains("application/auth-policy+xml") && !ct_str.contains("application/xml") {
            return xcap_error_response(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "not-xml-content-type",
                Some("Expected application/auth-policy+xml"),
            );
        }
    }

    // Parse the incoming XML
    let policy = match parse_pres_rules(&body) {
        Ok(p) => p,
        Err(e) => {
            return xcap_error_response(
                StatusCode::BAD_REQUEST,
                "not-well-formed",
                Some(&format!("Invalid pres-rules XML: {e}")),
            );
        }
    };

    // Check If-Match against existing policy
    let existing = state
        .manager
        .get_auth_policy_for_tenant(&xui, &state.default_tenant_id);
    if let Some(ref existing_policy) = existing {
        let existing_xml = generate_pres_rules(existing_policy);
        let current_etag = compute_etag(&existing_xml);
        if let Err(resp) = check_if_match(&headers, &current_etag) {
            return *resp;
        }
    } else if headers.contains_key(header::IF_MATCH) {
        return (StatusCode::PRECONDITION_FAILED, "Document does not exist").into_response();
    }

    let is_create = existing.is_none();
    state
        .manager
        .set_auth_policy(&xui, policy.clone(), &state.default_tenant_id);

    let response_xml = generate_pres_rules(&policy);
    let etag = compute_etag(&response_xml);

    let status = if is_create {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };

    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/auth-policy+xml")
        .header(header::ETAG, &etag)
        .body(Body::from(response_xml))
        .unwrap()
}

async fn delete_pres_rules(
    State(state): State<Arc<AppState>>,
    Path(xui): Path<String>,
    headers: HeaderMap,
) -> Response {
    // Check If-Match against existing policy
    let existing = state
        .manager
        .get_auth_policy_for_tenant(&xui, &state.default_tenant_id);
    match existing {
        Some(ref existing_policy) => {
            let existing_xml = generate_pres_rules(existing_policy);
            let current_etag = compute_etag(&existing_xml);
            if let Err(resp) = check_if_match(&headers, &current_etag) {
                return *resp;
            }
        }
        None => return StatusCode::NOT_FOUND.into_response(),
    }

    state
        .manager
        .remove_auth_policy_for_tenant(&xui, &state.default_tenant_id);
    StatusCode::OK.into_response()
}

// ============================================================================
// Serialization helpers for resource lists
// ============================================================================

/// Encode a list of `ResourceListXml` as JSON for `entries_json` storage.
fn encode_resource_lists(lists: &[ResourceListXml]) -> String {
    // Store as JSON array of {name, entries: [{uri, display_name}]}
    let serializable: Vec<serde_json::Value> = lists
        .iter()
        .map(|list| {
            serde_json::json!({
                "name": list.name,
                "entries": list.entries.iter().map(|e| {
                    serde_json::json!({
                        "uri": e.uri,
                        "display_name": e.display_name,
                    })
                }).collect::<Vec<_>>(),
            })
        })
        .collect();
    serde_json::to_string(&serializable).unwrap_or_else(|_| "[]".to_string())
}

/// Decode `entries_json` from a `StoredResourceList` into `Vec<ResourceListXml>`.
fn decode_resource_lists(stored: &StoredResourceList) -> Result<Vec<ResourceListXml>, String> {
    let values: Vec<serde_json::Value> =
        serde_json::from_str(&stored.entries_json).map_err(|e| e.to_string())?;

    let mut lists = Vec::new();
    for val in values {
        let name = val
            .get("name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let entries_val = val.get("entries").and_then(|v| v.as_array());
        let entries = entries_val
            .map(|arr| {
                arr.iter()
                    .filter_map(|e| {
                        let uri = e.get("uri")?.as_str()?.to_string();
                        let display_name = e
                            .get("display_name")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        Some(ResourceListEntryXml { uri, display_name })
                    })
                    .collect()
            })
            .unwrap_or_default();
        lists.push(ResourceListXml { name, entries });
    }

    Ok(lists)
}
