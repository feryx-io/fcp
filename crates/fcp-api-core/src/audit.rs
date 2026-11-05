//! Admin audit log middleware.
//!
//! Applied inside the bearer-auth layer on the `/api/v1` routers, so it sees
//! the caller's [`AuthIdentity`] and the matched route template. Every
//! mutating request (anything but `GET`/`HEAD`/`OPTIONS`), every export and
//! every read of a call's content is written to the compliance store as an
//! [`AuditEvent`]: who, which tenant, which route, the status the request
//! ended with, and the request's `X-Request-Id`, which the deletions it
//! made carry too (#342).
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

/// Which requests are worth an audit row: changes, and reads that take
/// data away or hear a call's content — exports, subject bundles, device
/// captures, recordings, voicemail audio and SIP traces. A person's reads
/// of their own (`/me/…`) are not audited.
pub fn should_audit(method: &Method, path: &str) -> bool {
    match *method {
        Method::GET => {
            let route = path.strip_prefix("/api/v1").unwrap_or(path);
            !route.starts_with("/me/")
                && (route.ends_with("/export")
                    || (route.starts_with("/compliance/subject-requests/")
                        && route.ends_with("/bundle"))
                    || (route.starts_with("/captures/") && route.ends_with("/file"))
                    || (route.starts_with("/recordings/") && route.ends_with("/download"))
                    || route.starts_with("/voicemail/messages/")
                    || (route.starts_with("/voicemail/mailboxes/")
                        && route.contains("/greetings/")
                        && route.ends_with("/audio"))
                    || (route.starts_with("/calls/") && route.ends_with("/sip-messages")))
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

/// The tenant a request acted on, put on its response by a handler that
/// knows better than the route (a tenant just made), or by a platform-only
/// router (`require_platform_admin`): the audit row is filed under it
/// (docs/TENANT_SCOPE.md §3.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuditScope {
    Tenant(TenantId),
    /// A platform-level action: the row has no tenant.
    Platform,
}

/// Routes whose actions are the platform's, not any tenant's: the system's
/// encryption and services, the cluster, and firmware images (a tenant's
/// firmware targets are its own).
fn platform_route(template: &str) -> bool {
    let route = template.strip_prefix("/api/v1").unwrap_or(template);
    route.starts_with("/system/")
        || route.starts_with("/cluster/")
        || route == "/firmware"
        || (route.starts_with("/firmware/") && !route.starts_with("/firmware/targets"))
}

/// The tenant a `/tenants/:id…` route names.
fn tenant_route(template: &str, path: &str) -> Option<TenantId> {
    let route = template.strip_prefix("/api/v1").unwrap_or(template);
    route.strip_prefix("/tenants/:id")?;
    let names = template.trim_end_matches('/').split('/');
    let values = path.trim_end_matches('/').split('/');
    // Matched from the end, as `route_detail` does.
    let names: Vec<_> = names.collect();
    let values: Vec<_> = values.collect();
    let at = names.iter().position(|n| *n == ":id")?;
    let from_end = names.len() - at;
    let value = values.get(values.len().checked_sub(from_end)?)?;
    percent_decode(value).parse().ok()
}

/// The tenant an audit row is filed under: what the response says, else a
/// platform route's none, else the tenant a `/tenants/:id` route names,
/// else the tenant the request worked in — a platform administrator's
/// picked tenant (`x-fcp-tenant-id`) or their own, anyone else's own.
fn acted_on(
    scope: Option<AuditScope>,
    template: &str,
    path: &str,
    identity: Option<&AuthIdentity>,
    header_tenant: Option<TenantId>,
    sign_in_tenant: Option<TenantId>,
    default_tenant: TenantId,
) -> Option<TenantId> {
    match scope {
        Some(AuditScope::Tenant(t)) => return Some(t),
        Some(AuditScope::Platform) => return None,
        None => {}
    }
    let platform = identity.is_some_and(AuthIdentity::is_platform_admin);
    if platform && platform_route(template) {
        return None;
    }
    if let Some(t) = tenant_route(template, path) {
        return Some(t);
    }
    let own = identity.and_then(|i| i.tenant_id().copied());
    if platform {
        return header_tenant.or(own).or(Some(default_tenant));
    }
    own.or(sign_in_tenant)
        .or_else(|| identity.map(|_| default_tenant))
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
    let request_id = req
        .headers()
        .get(crate::shape::REQUEST_ID_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let identity = req.extensions().get::<AuthIdentity>().cloned();

    // Whatever the request deletes is the caller's doing, in the deletion
    // log as well as here, under the same request id.
    let deleter = fcp_core::compliance::Deleter::new(
        fcp_core::compliance::DeletionCause::Manual,
        identity
            .as_ref()
            .map(AuthIdentity::actor)
            .unwrap_or_else(|| "anonymous".to_string()),
    )
    .in_api_request(request_id.clone());
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
    let tenant_id = acted_on(
        response.extensions().get::<AuditScope>().copied(),
        &template,
        &path,
        identity.as_ref(),
        header_tenant,
        sign_in.as_ref().and_then(|s| s.tenant_id),
        state.default_tenant_id,
    );
    // Where the actor came from: their own tenant (none for the static
    // token), or the tenant a sign-in named.
    let actor_tenant_id = identity
        .as_ref()
        .and_then(|i| i.tenant_id().copied())
        .or_else(|| sign_in.as_ref().and_then(|s| s.tenant_id));

    let detail = route_detail(&method, &template, &path);
    let event = AuditEvent {
        id: 0,
        tenant_id,
        actor_tenant_id,
        actor,
        actor_kind: actor_kind.to_string(),
        method: method.to_string(),
        path,
        status: response.status().as_u16(),
        action: format!("{method} {template}"),
        detail,
        source_ip: ip,
        api_request_id: request_id,
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
    fn a_row_is_filed_under_the_tenant_acted_on() {
        let home: TenantId = "tenant-00000000-0000-0000-0000-00000000000a"
            .parse()
            .unwrap();
        let other: TenantId = "tenant-00000000-0000-0000-0000-00000000000b"
            .parse()
            .unwrap();
        let default = TenantId::nil();
        let platform = AuthIdentity::StaticToken;
        let tenant_admin = AuthIdentity::Session {
            username: "ops".into(),
            role: "tenant_admin".into(),
            tenant_id: Some(home),
            group_roles: Vec::new(),
            user_id: None,
            session_id: None,
        };
        let on = |scope, template: &str, path: &str, who: &AuthIdentity, header| {
            acted_on(scope, template, path, Some(who), header, None, default)
        };
        // A platform administrator's picked tenant, else their own.
        assert_eq!(
            on(
                None,
                "/api/v1/users",
                "/api/v1/users",
                &platform,
                Some(other)
            ),
            Some(other)
        );
        assert_eq!(
            on(None, "/api/v1/users", "/api/v1/users", &platform, None),
            Some(default)
        );
        // The tenant a tenant route names, whatever is picked.
        let path = format!("/api/v1/tenants/{other}");
        assert_eq!(
            on(None, "/api/v1/tenants/:id", &path, &platform, Some(home)),
            Some(other)
        );
        let path = format!("/api/v1/tenants/{other}/deletion");
        assert_eq!(
            on(None, "/api/v1/tenants/:id/deletion", &path, &platform, None),
            Some(other)
        );
        // The platform's own actions have no tenant.
        for route in [
            "/api/v1/system/encryption/rotate",
            "/api/v1/cluster/nodes",
            "/api/v1/firmware",
            "/api/v1/firmware/:id",
        ] {
            assert_eq!(
                on(None, route, route, &platform, Some(other)),
                None,
                "{route}"
            );
        }
        assert_eq!(
            on(
                None,
                "/api/v1/firmware/targets",
                "/api/v1/firmware/targets",
                &platform,
                Some(other)
            ),
            Some(other)
        );
        // What the response says wins.
        assert_eq!(
            on(
                Some(AuditScope::Tenant(other)),
                "/api/v1/tenants",
                "/api/v1/tenants",
                &platform,
                None
            ),
            Some(other)
        );
        assert_eq!(
            on(
                Some(AuditScope::Platform),
                "/api/v1/x",
                "/api/v1/x",
                &platform,
                Some(other)
            ),
            None
        );
        // Anyone else is in their own tenant: a header changes nothing.
        assert_eq!(
            on(
                None,
                "/api/v1/users",
                "/api/v1/users",
                &tenant_admin,
                Some(other)
            ),
            Some(home)
        );
    }

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
        assert!(should_audit(&Method::GET, "/api/v1/audit-log/export"));
        assert!(should_audit(
            &Method::GET,
            "/api/v1/compliance/deletions/export"
        ));
        assert!(!should_audit(&Method::GET, "/api/v1/audit-log"));
        // A call's content heard or read.
        assert!(should_audit(
            &Method::GET,
            "/api/v1/recordings/rec-1/download"
        ));
        assert!(!should_audit(&Method::GET, "/api/v1/recordings/rec-1"));
        assert!(should_audit(
            &Method::GET,
            "/api/v1/voicemail/messages/vm-1"
        ));
        assert!(should_audit(
            &Method::GET,
            "/api/v1/voicemail/mailboxes/mbx-1/greetings/busy/audio"
        ));
        assert!(!should_audit(
            &Method::GET,
            "/api/v1/voicemail/mailboxes/mbx-1/messages"
        ));
        assert!(should_audit(
            &Method::GET,
            "/api/v1/calls/call-1/sip-messages"
        ));
        // A person's own.
        assert!(!should_audit(
            &Method::GET,
            "/api/v1/me/voicemail/messages/vm-1/audio"
        ));
        assert!(!should_audit(&Method::GET, "/api/v1/me/contacts/export"));
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
