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
        Method::GET | Method::HEAD | Method::OPTIONS => path.ends_with("/cdrs/export"),
        _ => true,
    }
}

fn source_ip(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .or_else(|| headers.get("x-real-ip").and_then(|v| v.to_str().ok()))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
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

    let response = next.run(req).await;

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

    let (actor, actor_kind) = match &identity {
        Some(AuthIdentity::StaticToken) => ("static_token".to_string(), "static_token"),
        Some(AuthIdentity::ApiToken { .. }) => ("api_token".to_string(), "api_token"),
        Some(AuthIdentity::Session { username, .. }) => (username.clone(), "session"),
        None => ("anonymous".to_string(), "anonymous"),
    };
    let tenant_id = identity
        .as_ref()
        .and_then(|i| i.tenant_id().copied())
        .or(header_tenant)
        .or_else(|| identity.as_ref().map(|_| state.default_tenant_id));

    let event = AuditEvent {
        id: 0,
        tenant_id,
        actor,
        actor_kind: actor_kind.to_string(),
        method: method.to_string(),
        path,
        status: response.status().as_u16(),
        action: format!("{method} {template}"),
        detail: None,
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
        assert!(!should_audit(&Method::GET, "/api/v1/cdrs"));
        assert!(!should_audit(&Method::OPTIONS, "/api/v1/users"));
    }
}
