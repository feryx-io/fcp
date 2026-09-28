//! A site's admin API (`docs/HYBRID_CONTROL_PLANE.md` §4): the objects the
//! link keeps in sync from the cloud — the tenant, its users, devices,
//! numbers, routes, flows and the rest of [`SYNCED_RESOURCES`] — are the
//! cloud's to change. When [`ApiState::managed_by`] names a cloud, a
//! write to one of them is refused here with that address, so an operator
//! at the site is sent where the change belongs and the site's copy never
//! drifts from the cloud's. Reads, and the runtime actions on those
//! resources (testing a trunk, an agent logging in or pausing), go through.

use crate::error::ApiError;
use crate::state::ApiState;
use axum::{
    extract::{Request, State},
    http::Method,
    middleware::Next,
    response::{IntoResponse, Response},
};

/// The API resources whose objects the link syncs, as path prefixes under
/// `/api/v1/`; every one is a table of `fcp_core::sync::SYNC_TABLES`, and
/// sites themselves, which only the cloud keeps.
pub const SYNCED_RESOURCES: &[&str] = &[
    "tenants",
    "sites",
    "users",
    "devices",
    "extensions",
    "line-bindings",
    "phone-numbers",
    "routing-rules",
    "lcr-routes",
    "trunks",
    "queue-configs",
    "flows",
    "prompts",
    "config-templates",
    "conferences/meetings",
    "locations",
    "emergency/config",
    "emergency/networks",
    "rate-plans",
    "schedules",
    "classes-of-service",
];

/// Whether `path` (relative to the v1 root, no leading slash) is a synced
/// resource, and which; a runtime action on one is not.
pub fn synced_resource(path: &str) -> Option<&'static str> {
    let path = path.trim_start_matches('/');
    let resource = SYNCED_RESOURCES
        .iter()
        .copied()
        .find(|r| path == *r || path.starts_with(&format!("{r}/")))?;
    // Actions on a trunk or a queue's agents are the site's own.
    if resource == "trunks" && path.ends_with("/test") {
        return None;
    }
    Some(resource)
}

/// Refuse a write to a synced resource when this deployment is a site.
pub async fn refuse_synced_writes(
    State(state): State<ApiState>,
    req: Request,
    next: Next,
) -> Response {
    if let Some(cloud) = &state.managed_by {
        let method = req.method();
        if method != Method::GET && method != Method::HEAD && method != Method::OPTIONS {
            let path = req.uri().path();
            let path = path.strip_prefix("/api/v1").unwrap_or(path);
            if let Some(resource) = synced_resource(path) {
                return ApiError::Forbidden(format!(
                    "{resource} are managed by the cloud at {cloud}; make this change there and the link brings it here"
                ))
                .into_response();
            }
        }
    }
    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::synced_resource;

    #[test]
    fn synced_resources_are_matched_by_prefix_and_runtime_actions_pass() {
        assert_eq!(synced_resource("/users"), Some("users"));
        assert_eq!(synced_resource("users/alice/password"), Some("users"));
        assert_eq!(
            synced_resource("conferences/meetings/x"),
            Some("conferences/meetings")
        );
        assert_eq!(synced_resource("conferences/live/room/dial-out"), None);
        assert_eq!(synced_resource("emergency/notify-call"), None);
        assert_eq!(
            synced_resource("emergency/config"),
            Some("emergency/config")
        );
        assert_eq!(synced_resource("trunks/t1/test"), None);
        assert_eq!(synced_resource("trunks/t1"), Some("trunks"));
        assert_eq!(synced_resource("queues/sales/agents/a1/pause"), None);
        assert_eq!(
            synced_resource("queue-configs/sales"),
            Some("queue-configs")
        );
        assert_eq!(synced_resource("userspace"), None);
        assert_eq!(
            synced_resource("classes-of-service/cos-1"),
            Some("classes-of-service")
        );
        assert_eq!(synced_resource("schedules"), Some("schedules"));
    }
}
