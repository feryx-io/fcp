//! What this node would do with a call, without placing it
//! (`docs/TENANT_TRUNKS.md` §12, phase 6d).
//!
//! `GET /routes/preview?tenant=…&to=…[&from=…][&source_ip=…][&location=…]` runs the same
//! decisions an INVITE meets, in the same order, against what this node
//! holds right now: emergency detection, the node's YAML policy, the
//! tenant's own rules, then the trunks — the rule's target, else the
//! tenant's own by its least-cost routes or ordinary choice, then the
//! platform's — each with its price and state. Nothing is sent, counted or
//! recorded. The admin API reaches it through one live call manager, so
//! the answer is a node's, not a copy of its logic.

use crate::health::AppState;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use fcp_core::TenantId;
use fcp_sbc_core::{RoutePrice, TrunkConfig, TrunkDirection};
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, Ipv4Addr};
use std::str::FromStr;
use std::sync::Arc;

#[derive(Debug, Deserialize)]
pub struct PreviewQuery {
    /// Whose call it is.
    pub tenant: String,
    /// What is dialled: a number (`+442079460000`) or a SIP URI.
    pub to: String,
    /// Who is calling, as a SIP URI or a user; rules and the YAML policy
    /// that match on the caller need it.
    pub from: Option<String>,
    /// Where the call comes from, for rules with source ranges.
    pub source_ip: Option<IpAddr>,
    /// Where the caller is, for a location's rules (docs/TENANT_SCOPE.md
    /// §3.6); a call has no device here to say.
    pub location: Option<String>,
}

/// One trunk the call would be offered, in the order they are tried.
#[derive(Debug, Serialize)]
pub struct PreviewRoute {
    pub trunk_id: String,
    pub name: String,
    /// `tenant` for the tenant's own trunk, `platform` for the node's.
    pub owner: &'static str,
    pub priority: u32,
    pub weight: u32,
    pub trunk_group: Option<String>,
    /// This node's own view of the trunk's health.
    pub health: String,
    /// What the destination costs on it, and by whose table; absent when
    /// the ordinary choice placed it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price: Option<RoutePrice>,
}

#[derive(Debug, Serialize)]
pub struct Decision {
    /// `allow`, `deny`, `error`, `no_match` or `not_applied`.
    pub decision: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// A `route_to_trunk` rule's target: `trunk <id>` or `trunk group <name>`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

impl Decision {
    fn of(decision: &'static str) -> Self {
        Self {
            decision,
            rule: None,
            reason: None,
            target: None,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Preview {
    pub node_id: String,
    pub tenant_id: String,
    /// The destination as the node matched it.
    pub destination: String,
    /// `live` or `test` when the number is an emergency number: it takes
    /// the emergency path, which no rule touches.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emergency: Option<String>,
    /// The node's own YAML policy (`[routing] rules_file`).
    pub policy: Decision,
    /// The tenant's own rules (`/v1/routing/rules`).
    pub rule: Decision,
    /// `routes`, `refused`, `unavailable` or `no_route`.
    pub outcome: &'static str,
    /// What the caller would hear when the call does not go out.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    /// The trunks in the order they would be tried, one at a time.
    pub routes: Vec<PreviewRoute>,
    /// Trunks the tenant's calls may use that would not be offered this
    /// call: not what a rule named, unhealthy, at capacity or disabled,
    /// or behind the least-cost cut. Shown so an empty `routes` explains
    /// itself.
    pub not_offered: Vec<PreviewRoute>,
}

/// A number becomes a URI the way a phone's INVITE names it; a URI is kept.
fn destination_uri(to: &str) -> String {
    let to = to.trim();
    if to.contains(':') {
        to.to_string()
    } else {
        format!("sip:{to}@preview.invalid")
    }
}

fn describe(
    state: &AppState,
    tenant: TenantId,
    trunk: &TrunkConfig,
    destination: &str,
    router: &fcp_sbc_core::TrunkRouter,
) -> PreviewRoute {
    PreviewRoute {
        trunk_id: trunk.trunk_id().to_string(),
        name: trunk.name.clone(),
        owner: if trunk.tenant_id.is_some() {
            "tenant"
        } else {
            "platform"
        },
        priority: trunk.priority,
        weight: trunk.weight,
        trunk_group: trunk.trunk_group.clone(),
        health: state
            .trunk_health
            .as_ref()
            .map(|h| {
                if !trunk.enabled {
                    "disabled".to_string()
                } else if h.is_healthy(&trunk.key()) {
                    "healthy".to_string()
                } else {
                    "down".to_string()
                }
            })
            .unwrap_or_else(|| "unknown".to_string()),
        price: router.price_for(Some(tenant), trunk, destination),
    }
}

pub async fn route_preview_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<PreviewQuery>,
) -> impl IntoResponse {
    let Ok(tenant) = TenantId::from_str(query.tenant.trim()) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "tenant is not a tenant id"})),
        )
            .into_response();
    };
    let (Some(router), Some(trunk_set)) = (&state.trunk_router, &state.trunk_set) else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "this node routes by no trunks"})),
        )
            .into_response();
    };
    let destination = destination_uri(&query.to);
    let caller = query
        .from
        .as_deref()
        .map(str::trim)
        .filter(|f| !f.is_empty())
        .map(|f| {
            if f.contains(':') {
                f.to_string()
            } else {
                format!("sip:{f}@preview.invalid")
            }
        })
        .unwrap_or_default();
    let source_ip = query.source_ip.unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
    let location = match query
        .location
        .as_deref()
        .map(str::trim)
        .filter(|l| !l.is_empty())
    {
        Some(location) => match fcp_core::LocationId::from_str(location) {
            Ok(location) => Some(location),
            Err(_) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": "location is not a location id"})),
                )
                    .into_response();
            }
        },
        None => None,
    };
    let direction = TrunkDirection::Upstream;

    let mut preview = Preview {
        node_id: state.node_id.clone(),
        tenant_id: tenant.to_string(),
        destination: destination.clone(),
        emergency: None,
        policy: Decision::of("not_applied"),
        rule: Decision::of("not_applied"),
        outcome: "routes",
        status: None,
        routes: Vec::new(),
        not_offered: Vec::new(),
    };
    let describe = |t: &TrunkConfig| describe(&state, tenant, t, &destination, router);

    // Emergency first: detection comes before any policy, and the
    // emergency path picks its own trunks.
    let config = fcp_sip::emergency::config_for(Some(&state.storage), &tenant).await;
    if let Some(kind) = config.classify(&destination) {
        preview.emergency = Some(kind.to_string());
        let trunks = router.select_emergency_trunks_ordered(Some(tenant), direction, &destination);
        preview.routes = trunks.iter().map(|t| describe(t)).collect();
        if preview.routes.is_empty() {
            preview.outcome = "unavailable";
            preview.status = Some(503);
        }
        return Json(preview).into_response();
    }

    // The node's own policy.
    if let Some(service) = &state.routing_service {
        preview.policy = match service
            .evaluate_call(&caller, &destination, &tenant, source_ip)
            .await
        {
            Ok(decision) if decision.action.is_allow() => Decision {
                rule: decision.matched_rule,
                reason: Some(decision.reason),
                ..Decision::of("allow")
            },
            Ok(decision) => {
                preview.outcome = "refused";
                preview.status = Some(403);
                Decision {
                    rule: decision.matched_rule,
                    reason: Some(decision.reason),
                    ..Decision::of("deny")
                }
            }
            Err(e) => {
                preview.outcome = "refused";
                preview.status = Some(500);
                Decision {
                    reason: Some(e.to_string()),
                    ..Decision::of("error")
                }
            }
        };
        if preview.status.is_some() {
            return Json(preview).into_response();
        }
    }

    // The tenant's own rules.
    let mut target = None;
    if let Some(rules) = &state.tenant_rules {
        let facts = fcp_policy::CallFacts {
            caller: &caller,
            destination: &destination,
            source_ip,
            location,
        };
        preview.rule = match rules.evaluate(&tenant, &facts) {
            fcp_policy::RuleOutcome::NoMatch => Decision::of("no_match"),
            fcp_policy::RuleOutcome::Allow { rule } => Decision {
                rule: Some(rule),
                ..Decision::of("allow")
            },
            fcp_policy::RuleOutcome::Deny { rule, reason } => {
                preview.outcome = "refused";
                preview.status = Some(403);
                Decision {
                    rule: Some(rule),
                    reason: Some(reason),
                    ..Decision::of("deny")
                }
            }
            fcp_policy::RuleOutcome::Route { rule, target: t } => {
                let described = t.to_string();
                target = Some(t);
                Decision {
                    rule: Some(rule),
                    target: Some(described),
                    ..Decision::of("route")
                }
            }
        };
        if preview.status.is_some() {
            return Json(preview).into_response();
        }
    }

    // The trunks.
    let (trunks, known) = match &target {
        Some(target) => {
            router.select_targeted_trunks(Some(tenant), direction, &destination, target)
        }
        None => {
            let trunks = router.select_trunks_ordered(Some(tenant), direction, &destination);
            let known = router.has_configured_route(Some(tenant), direction, &destination);
            (trunks, known)
        }
    };
    preview.routes = trunks.iter().map(|t| describe(t)).collect();
    preview.not_offered = trunk_set
        .for_tenant(Some(tenant))
        .iter()
        .filter(|t| !trunks.iter().any(|o| o.key() == t.key()))
        .map(|t| describe(t))
        .collect();
    if preview.routes.is_empty() {
        (preview.outcome, preview.status) = if known {
            ("unavailable", Some(503))
        } else {
            ("no_route", Some(404))
        };
    }
    Json(preview).into_response()
}

#[cfg(test)]
mod tests {
    use super::destination_uri;

    #[test]
    fn a_number_is_named_as_a_phone_would_name_it() {
        assert_eq!(
            destination_uri(" +442079460000 "),
            "sip:+442079460000@preview.invalid"
        );
        assert_eq!(destination_uri("sip:911@pbx"), "sip:911@pbx");
        assert_eq!(destination_uri("tel:+15125550100"), "tel:+15125550100");
    }
}
