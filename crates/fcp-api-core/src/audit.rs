//! Admin audit log middleware.
//!
//! Applied inside the bearer-auth layer on the `/api/v1` routers, so it sees
//! the caller's [`AuthIdentity`] and the matched route template. Every
//! mutating request (anything but `GET`/`HEAD`/`OPTIONS`) and every CDR
//! export is written to the compliance store as an [`AuditEvent`]:
//! who, which tenant, which route, and the status the request ended with.
//! Login attempts on the pre-auth router are recorded with actor
//! `anonymous`, so failed logins leave a trace too.
//!
//! Recording is best-effort and asynchronous: a storage failure is logged
//! and never affects the response.

use crate::auth::{AuthIdentity, PLATFORM_TENANT_HEADER};
use crate::state::ApiState;
use axum::{
    extract::{MatchedPath, OriginalUri, Request, State},
    http::{HeaderMap, Method},
    middleware::Next,
    response::Response,
};
use fcp_core::compliance::AuditEvent;
use fcp_core::TenantId;
use std::time::SystemTime;
use tracing::warn;

/// Which requests are worth an audit row.
pub fn should_audit(method: &Method, path: &str) -> bool {
    match *method {
        Method::GET => {
            path.ends_with("/cdrs/export")
                || (path.contains("/compliance/subject-requests/") && path.ends_with("/bundle"))
                || (path.contains("/captures/") && path.ends_with("/file"))
        }
        Method::HEAD | Method::OPTIONS => false,
        _ => true,
    }
}

pub(crate) fn source_ip(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .or_else(|| headers.get("x-real-ip").and_then(|v| v.to_str().ok()))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// What a deletion named: the route's parameters (`{"params": {"id":
/// "…"}}`), matched from the end of the path so a nested router's prefix
/// does not shift them. Other requests carry no detail.
pub fn route_detail(method: &Method, template: &str, path: &str) -> Option<serde_json::Value> {
    if *method != Method::DELETE {
        return None;
    }
    let names = template.trim_end_matches('/').rsplit('/');
    let values = path.trim_end_matches('/').rsplit('/');
    let mut params = serde_json::Map::new();
    for (name, value) in names.zip(values) {
        if let Some(name) = name.strip_prefix(':') {
            let value = percent_decode(value);
            params.insert(name.to_string(), serde_json::Value::String(value));
        }
    }
    (!params.is_empty()).then(|| serde_json::json!({ "params": params }))
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&text[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Who a sign-in attempt named, put on its response by the sign-in routes
/// (`docs/IDENTITY_AND_DEVICES.md` phase 1): the request carries no
/// identity, and the audit row should still say who tried, and in which
/// tenant when the username was found.
#[derive(Clone, Debug)]
pub struct SignInSubject {
    pub username: String,
    pub tenant_id: Option<TenantId>,
}

/// Middleware: record the request after it completes.
pub async fn audit_requests(State(state): State<ApiState>, req: Request, next: Next) -> Response {
    let method = req.method().clone();
    // Inside a nested router `req.uri()` is relative to the nest; the
    // original URI keeps the full path.
    let path = req
        .extensions()
        .get::<OriginalUri>()
        .map(|u| u.path().to_string())
        .unwrap_or_else(|| req.uri().path().to_string());
    let template = req
        .extensions()
        .get::<MatchedPath>()
        .map(|m| m.as_str().to_string())
        .unwrap_or_else(|| path.clone());
    let header_tenant = req
        .headers()
        .get(PLATFORM_TENANT_HEADER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<TenantId>().ok());
    let ip = source_ip(req.headers());
    let identity = req.extensions().get::<AuthIdentity>().cloned();

    // Whatever the request deletes is the caller's doing, in the deletion
    // log as well as here.
    let deleter = fcp_core::compliance::Deleter::new(
        fcp_core::compliance::DeletionCause::Manual,
        identity
            .as_ref()
            .map(AuthIdentity::actor)
            .unwrap_or_else(|| "anonymous".to_string()),
    );
    let response = fcp_core::compliance::deleting_as(deleter, next.run(req)).await;

    if !should_audit(&method, &path) {
        return response;
    }
    let Some(store) = state
        .storage
        .as_ref()
        .and_then(|s| s.compliance_store())
        .cloned()
    else {
        return response;
    };

    let sign_in = response.extensions().get::<SignInSubject>().cloned();
    let actor = identity
        .as_ref()
        .map(AuthIdentity::actor)
        .or_else(|| sign_in.as_ref().map(|s| s.username.clone()))
        .unwrap_or_else(|| "anonymous".to_string());
    let actor_kind = match (&identity, &sign_in) {
        (Some(AuthIdentity::StaticToken), _) => "static_token",
        (Some(AuthIdentity::ApiToken { .. }), _) => "api_token",
        (Some(AuthIdentity::Personal { .. }), _) => "personal_token",
        (Some(AuthIdentity::Session { .. }), _) => "session",
        (None, Some(_)) => "sign_in",
        (None, None) => "anonymous",
    };
    let tenant_id = identity
        .as_ref()
        .and_then(|i| i.tenant_id().copied())
        .or_else(|| sign_in.as_ref().and_then(|s| s.tenant_id))
        .or(header_tenant)
        .or_else(|| identity.as_ref().map(|_| state.default_tenant_id));

    let detail = route_detail(&method, &template, &path);
    let event = AuditEvent {
        id: 0,
        tenant_id,
        actor,
        actor_kind: actor_kind.to_string(),
        method: method.to_string(),
        path,
        status: response.status().as_u16(),
        action: format!("{method} {template}"),
        detail,
        source_ip: ip,
        occurred_at: SystemTime::now(),
    };
    tokio::spawn(async move {
        if let Err(e) = store.record_audit_event(&event).await {
            warn!(error = %e, action = %event.action, "Failed to write admin audit event");
        }
    });
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_mutations_and_exports_are_audited() {
        assert!(should_audit(&Method::POST, "/api/v1/users"));
        assert!(should_audit(&Method::DELETE, "/api/v1/users/alice"));
        assert!(should_audit(&Method::GET, "/api/v1/cdrs/export"));
        assert!(should_audit(
            &Method::GET,
            "/api/v1/compliance/subject-requests/sreq-1/bundle"
        ));
        assert!(!should_audit(
            &Method::GET,
            "/api/v1/compliance/subject-requests/sreq-1"
        ));
        assert!(should_audit(&Method::GET, "/api/v1/captures/cap-1/file"));
        assert!(!should_audit(&Method::GET, "/api/v1/captures"));
        assert!(!should_audit(&Method::GET, "/api/v1/cdrs"));
        assert!(!should_audit(&Method::OPTIONS, "/api/v1/users"));
    }

    #[test]
    fn a_deletion_names_what_it_deleted() {
        assert_eq!(
            route_detail(
                &Method::DELETE,
                "/api/v1/voicemail/mailboxes/:id/greetings/:greeting_type",
                "/api/v1/voicemail/mailboxes/mbx-1/greetings/busy",
            ),
            Some(serde_json::json!({ "params": { "id": "mbx-1", "greeting_type": "busy" } }))
        );
        // A template without the nest's prefix still lines up.
        assert_eq!(
            route_detail(&Method::DELETE, "/users/:username", "/api/v1/users/j%20doe"),
            Some(serde_json::json!({ "params": { "username": "j doe" } }))
        );
        assert_eq!(
            route_detail(&Method::PUT, "/users/:username", "/api/v1/users/alice"),
            None
        );
        assert_eq!(
            route_detail(&Method::DELETE, "/cache", "/api/v1/cache"),
            None
        );
        assert_eq!(percent_decode("100%"), "100%");
    }
}
