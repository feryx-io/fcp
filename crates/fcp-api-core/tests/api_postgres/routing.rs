//! Routing rules, least-cost routes, trunks, rate plans and classes of
//! service, route by route (docs/DEEP_TESTING.md D3b).

use super::*;

impl App {
    async fn create_trunk_in(&self, tenant: TenantId, body: Value) -> String {
        let res = self
            .admin_in(tenant, Method::POST, "/api/v1/trunks", Some(body))
            .await;
        assert_eq!(res.status, StatusCode::CREATED, "{}", res.text);
        res.json["id"].as_str().expect("trunk id").to_string()
    }
}

/// A rule is read, changed and deleted by id in its tenant: a change is
/// checked as a new rule is, and another tenant sees none of it.
#[tokio::test]
async fn a_routing_rule_is_read_changed_and_deleted_in_its_tenant() {
    let app = require_app!(DeploymentMode::Carrier);
    let ours = app
        .create_trunk_in(
            app.tenant_a,
            json!({ "name": "intl", "hosts": ["sip.carrier.example.net"] }),
        )
        .await;
    let theirs = app
        .create_trunk_in(
            app.tenant_b,
            json!({ "name": "intl", "hosts": ["sip.other.example.net"] }),
        )
        .await;
    let created = app
        .admin(
            Method::POST,
            "/api/v1/routing/rules",
            Some(
                json!({ "name": "no premium", "priority": 10, "enabled": true,
                         "action": "deny", "destination_pattern": "^\\+1900",
                         "reason": "premium rate" }),
            ),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.text);
    let id = created.json["id"].as_str().unwrap().to_string();
    let path = format!("/api/v1/routing/rules/{id}");

    let got = app.admin(Method::GET, &path, None).await;
    assert_eq!(got.status, StatusCode::OK, "{}", got.text);
    assert_eq!(got.json["name"], "no premium");
    assert_eq!(got.json["destination_pattern"], "^\\+1900");
    let from_b = app.admin_in(app.tenant_b, Method::GET, &path, None).await;
    assert_eq!(from_b.status, StatusCode::NOT_FOUND, "{}", from_b.text);
    let unknown = app
        .admin(Method::GET, "/api/v1/routing/rules/no-such-rule", None)
        .await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND, "{}", unknown.text);

    // A change is checked as a new rule is.
    for (body, needle) in [
        (json!({ "destination_pattern": "([" }), "destination"),
        (json!({ "source_pattern": "([" }), "source"),
        (json!({ "action": "sideways" }), "sideways"),
        (
            json!({ "action": "route_to_trunk", "target_trunk_id": theirs }),
            "not one of this tenant's trunks",
        ),
    ] {
        let res = app.admin(Method::PATCH, &path, Some(body.clone())).await;
        assert_eq!(res.status, StatusCode::BAD_REQUEST, "{body}: {}", res.text);
        assert!(res.text.contains(needle), "{needle} in {}", res.text);
    }
    let unchanged = app.admin(Method::GET, &path, None).await;
    assert_eq!(
        unchanged.json["action"], got.json["action"],
        "{}",
        unchanged.text
    );

    let b_patch = app
        .admin_in(
            app.tenant_b,
            Method::PATCH,
            &path,
            Some(json!({ "enabled": false })),
        )
        .await;
    assert_eq!(b_patch.status, StatusCode::NOT_FOUND, "{}", b_patch.text);

    // A good change is kept, what it leaves out stays, and an empty pattern
    // clears it.
    let mut events = fcp_events::EventSubscriber::subscribe(
        app.event_bus.as_ref(),
        fcp_events::EventFilter::new(),
    )
    .await
    .expect("subscribe");
    let changed = app
        .admin(
            Method::PATCH,
            &path,
            Some(json!({ "action": "route_to_trunk", "target_trunk_id": ours,
                         "priority": 5, "destination_pattern": "" })),
        )
        .await;
    assert_eq!(changed.status, StatusCode::OK, "{}", changed.text);
    assert_eq!(changed.json["priority"], 5);
    assert_eq!(changed.json["target_trunk_id"], ours.as_str());
    assert_eq!(changed.json["destination_pattern"], Value::Null);
    assert_eq!(changed.json["name"], "no premium");
    let announced =
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match events.next().await {
                    Some(PlatformEvent::System(SystemEvent::ConfigChanged {
                        entity_type, ..
                    })) if entity_type == "routing_rule" => return true,
                    Some(_) => continue,
                    None => return false,
                }
            }
        })
        .await;
    assert_eq!(announced, Ok(true), "the change is announced");
    let read_back = app.admin(Method::GET, &path, None).await;
    assert_eq!(read_back.json["priority"], 5, "{}", read_back.text);

    let b_delete = app
        .admin_in(app.tenant_b, Method::DELETE, &path, None)
        .await;
    assert_eq!(b_delete.status, StatusCode::NOT_FOUND, "{}", b_delete.text);
    let deleted = app.admin(Method::DELETE, &path, None).await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT, "{}", deleted.text);
    let after = app.admin(Method::GET, &path, None).await;
    assert_eq!(after.status, StatusCode::NOT_FOUND, "{}", after.text);
    app.finish().await;
}

/// A least-cost route is deleted by its own tenant only.
#[tokio::test]
async fn a_least_cost_route_is_deleted_by_its_tenant() {
    let app = require_app!(DeploymentMode::Carrier);
    let trunk = app
        .create_trunk_in(
            app.tenant_a,
            json!({ "name": "cheap", "hosts": ["sip.carrier.example.net"] }),
        )
        .await;
    let made = app
        .admin(
            Method::POST,
            "/api/v1/lcr-routes",
            Some(json!({ "prefix": "+33", "trunk_id": trunk, "cost_per_minute": 0.02 })),
        )
        .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.text);
    let path = format!("/api/v1/lcr-routes/{}", made.json["id"].as_str().unwrap());

    let b_delete = app
        .admin_in(app.tenant_b, Method::DELETE, &path, None)
        .await;
    assert_eq!(b_delete.status, StatusCode::NOT_FOUND, "{}", b_delete.text);
    let deleted = app.admin(Method::DELETE, &path, None).await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT, "{}", deleted.text);
    let after = app.admin(Method::GET, &path, None).await;
    assert_eq!(after.status, StatusCode::NOT_FOUND, "{}", after.text);
    let again = app.admin(Method::DELETE, &path, None).await;
    assert_eq!(again.status, StatusCode::NOT_FOUND, "{}", again.text);
    app.finish().await;
}

/// A trunk's copy is the tenant's, disabled, with the original's settings;
/// its stats count the tenant's own calls on it only; its test sends an
/// OPTIONS that a listening carrier answers.
#[tokio::test]
async fn a_trunk_is_copied_counted_and_tested() {
    let app = require_app!(DeploymentMode::Carrier);

    // A carrier that answers OPTIONS.
    let carrier = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let port = carrier.local_addr().unwrap().port();
    let answered = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = Arc::clone(&answered);
    tokio::spawn(async move {
        let mut buf = vec![0u8; 4096];
        while let Ok((n, from)) = carrier.recv_from(&mut buf).await {
            let request = String::from_utf8_lossy(&buf[..n]).into_owned();
            if !request.starts_with("OPTIONS ") {
                continue;
            }
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let mut response = String::from("SIP/2.0 200 OK\r\n");
            for line in request.lines().filter(|l| {
                ["via:", "from:", "to:", "call-id:", "cseq:"]
                    .iter()
                    .any(|h| l.to_ascii_lowercase().starts_with(h))
            }) {
                response.push_str(line);
                response.push_str("\r\n");
            }
            response.push_str("Content-Length: 0\r\n\r\n");
            let _ = carrier.send_to(response.as_bytes(), from).await;
        }
    });

    let trunk = app
        .create_trunk_in(
            app.tenant_a,
            json!({ "name": "carrier", "hosts": [format!("127.0.0.1:{port}")],
                    "auth_username": "acct", "auth_password": "s3cret", "priority": 7 }),
        )
        .await;
    let same_name = app
        .create_trunk_in(
            app.tenant_b,
            json!({ "name": "carrier", "hosts": ["sip.other.example.net"] }),
        )
        .await;
    let path = format!("/api/v1/trunks/{trunk}");

    // The test reaches the carrier.
    let tested = app.admin(Method::POST, &format!("{path}/test"), None).await;
    assert_eq!(tested.status, StatusCode::OK, "{}", tested.text);
    assert_eq!(tested.json["success"], true, "{}", tested.text);
    assert_eq!(tested.json["trunk_name"], "carrier");
    assert!(answered.load(std::sync::atomic::Ordering::SeqCst) >= 1);
    let not_b = app
        .admin_in(app.tenant_b, Method::POST, &format!("{path}/test"), None)
        .await;
    assert_eq!(not_b.status, StatusCode::NOT_FOUND, "{}", not_b.text);
    let hostless = app
        .create_trunk_in(app.tenant_a, json!({ "name": "nowhere", "hosts": [] }))
        .await;
    let no_hosts = app
        .admin(
            Method::POST,
            &format!("/api/v1/trunks/{hostless}/test"),
            None,
        )
        .await;
    assert_eq!(
        no_hosts.status,
        StatusCode::BAD_REQUEST,
        "{}",
        no_hosts.text
    );

    // The copy: the tenant's, disabled, named as a copy, settings kept.
    let copy = app
        .admin(Method::POST, &format!("{path}/clone"), None)
        .await;
    assert_eq!(copy.status, StatusCode::CREATED, "{}", copy.text);
    assert_ne!(copy.json["id"], trunk.as_str());
    assert_eq!(copy.json["name"], "carrier (copy)");
    assert_eq!(copy.json["enabled"], false);
    assert_eq!(copy.json["priority"], 7);
    assert_eq!(copy.json["hosts"], json!([format!("127.0.0.1:{port}")]));
    assert_eq!(copy.json["tenant_id"], app.tenant_a.to_string());
    assert!(!copy.text.contains("s3cret"), "{}", copy.text);
    let copy_id = copy.json["id"].as_str().unwrap();
    let stored = app
        .admin(Method::GET, &format!("/api/v1/trunks/{copy_id}"), None)
        .await;
    assert_eq!(stored.status, StatusCode::OK, "{}", stored.text);
    let b_copy = app
        .admin_in(app.tenant_b, Method::POST, &format!("{path}/clone"), None)
        .await;
    assert_eq!(b_copy.status, StatusCode::NOT_FOUND, "{}", b_copy.text);

    // Stats: the tenant's calls on its trunk, and not the other tenant's
    // on a trunk of the same name.
    let on = |tenant: TenantId, answered: Option<u64>| {
        let mut cdr = cdr_record(
            tenant,
            answered,
            None,
            SystemTime::now() - Duration::from_secs(600),
        );
        cdr.gateway_id = Some("carrier".to_string());
        cdr
    };
    for cdr in [
        on(app.tenant_a, Some(60_000)),
        on(app.tenant_a, None),
        on(app.tenant_b, Some(30_000)),
        on(app.tenant_b, Some(30_000)),
        on(app.tenant_b, Some(30_000)),
    ] {
        app.storage.insert_cdr(cdr).await.expect("insert cdr");
    }
    let stats = app.admin(Method::GET, &format!("{path}/stats"), None).await;
    assert_eq!(stats.status, StatusCode::OK, "{}", stats.text);
    assert_eq!(stats.json["total_calls_24h"], 2, "{}", stats.text);
    assert_eq!(stats.json["answered_calls_24h"], 1, "{}", stats.text);
    assert_eq!(stats.json["asr"], 50.0, "{}", stats.text);
    assert_eq!(stats.json["acd_secs"], 60.0, "{}", stats.text);
    assert_eq!(stats.json["pdd_ms"], 2000.0, "{}", stats.text);
    let hourly: u64 = stats.json["hourly_volume"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap())
        .sum();
    assert_eq!(hourly, 2, "{}", stats.text);
    let theirs = app
        .admin_in(
            app.tenant_b,
            Method::GET,
            &format!("/api/v1/trunks/{same_name}/stats"),
            None,
        )
        .await;
    assert_eq!(theirs.json["total_calls_24h"], 3, "{}", theirs.text);
    let not_theirs = app
        .admin_in(app.tenant_b, Method::GET, &format!("{path}/stats"), None)
        .await;
    assert_eq!(
        not_theirs.status,
        StatusCode::NOT_FOUND,
        "{}",
        not_theirs.text
    );
    app.finish().await;
}

/// The tenant's rate plans are listed, and only its own.
#[tokio::test]
async fn rate_plans_are_listed_in_their_tenant() {
    let app = require_app!();
    for name in ["Retail", "Wholesale"] {
        let res = app
            .admin(
                Method::POST,
                "/api/v1/rate-plans",
                Some(json!({ "name": name, "currency": "USD" })),
            )
            .await;
        assert_eq!(res.status, StatusCode::CREATED, "{}", res.text);
    }
    let theirs = app
        .admin_in(
            app.tenant_b,
            Method::POST,
            "/api/v1/rate-plans",
            Some(json!({ "name": "Theirs", "currency": "EUR" })),
        )
        .await;
    assert_eq!(theirs.status, StatusCode::CREATED, "{}", theirs.text);

    let listed = app.admin(Method::GET, "/api/v1/rate-plans", None).await;
    assert_eq!(listed.status, StatusCode::OK, "{}", listed.text);
    let names: Vec<&str> = listed
        .json
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["Retail", "Wholesale"], "{}", listed.text);
    let page = app
        .admin(Method::GET, "/api/v1/rate-plans?limit=1", None)
        .await;
    assert_eq!(page.json.as_array().unwrap().len(), 1, "{}", page.text);
    assert!(page.body["next_cursor"].is_string(), "{}", page.text);
    app.finish().await;
}

/// A class of service is replaced whole by a PUT, checked as a new one is,
/// and only in its tenant.
#[tokio::test]
async fn a_class_of_service_is_replaced_whole() {
    let app = require_app!();
    let make = |body: Value| {
        let app = &app;
        async move {
            let res = app
                .admin(Method::POST, "/api/v1/classes-of-service", Some(body))
                .await;
            assert_eq!(res.status, StatusCode::CREATED, "{}", res.text);
            res.json["id"].as_str().unwrap().to_string()
        }
    };
    let local = make(json!({ "name": "Local", "allowed": ["internal"], "is_default": true })).await;
    make(json!({ "name": "Lobby", "allowed": ["internal"] })).await;
    let path = format!("/api/v1/classes-of-service/{local}");

    let taken = app
        .admin(Method::PUT, &path, Some(json!({ "name": "lobby" })))
        .await;
    assert_eq!(taken.status, StatusCode::CONFLICT, "{}", taken.text);

    let replaced = app
        .admin(
            Method::PUT,
            &path,
            Some(
                json!({ "name": "Local+", "allowed": ["internal", "national"],
                         "max_calls": 4, "caller_id": "withheld" }),
            ),
        )
        .await;
    assert_eq!(replaced.status, StatusCode::OK, "{}", replaced.text);
    assert_eq!(replaced.json["name"], "Local+");
    assert_eq!(replaced.json["max_calls"], 4);
    assert_eq!(replaced.json["caller_id"], "withheld");
    // Not said, so not the default any more: a PUT is the whole class.
    assert_eq!(replaced.json["is_default"], false, "{}", replaced.text);
    let allowed = replaced.json["allowed"].as_array().unwrap();
    assert!(allowed.contains(&json!("national")), "{}", replaced.text);
    assert!(allowed.contains(&json!("emergency")), "{}", replaced.text);
    let read_back = app.admin(Method::GET, &path, None).await;
    assert_eq!(read_back.json["name"], "Local+", "{}", read_back.text);

    let unlimited = app
        .admin(
            Method::PUT,
            &path,
            Some(json!({ "name": "Local+", "max_calls": null })),
        )
        .await;
    assert_eq!(unlimited.status, StatusCode::OK, "{}", unlimited.text);
    assert!(unlimited.json["max_calls"].is_null(), "{}", unlimited.text);

    let from_b = app
        .admin_in(
            app.tenant_b,
            Method::PUT,
            &path,
            Some(json!({ "name": "Mine now" })),
        )
        .await;
    assert_eq!(from_b.status, StatusCode::NOT_FOUND, "{}", from_b.text);
    let bad_id = app
        .admin(
            Method::PUT,
            "/api/v1/classes-of-service/not-an-id",
            Some(json!({ "name": "x" })),
        )
        .await;
    assert_eq!(bad_id.status, StatusCode::BAD_REQUEST, "{}", bad_id.text);
    app.finish().await;
}

/// A rule may be for one of the tenant's locations (docs/TENANT_SCOPE.md
/// §3.6): another tenant's location is refused, the list filters by it,
/// it can be cleared, and the location cannot be deleted under it.
#[tokio::test]
async fn a_routing_rule_is_for_one_of_the_tenants_locations() {
    let app = require_app!();
    let location = |tenant: TenantId, name: &'static str| {
        let app = &app;
        async move {
            let res = app
                .admin_in(
                    tenant,
                    Method::POST,
                    "/api/v1/locations",
                    Some(json!({ "name": name, "city": "Troy", "country": "US" })),
                )
                .await;
            assert_eq!(res.status, StatusCode::CREATED, "{}", res.text);
            res.json["id"].as_str().unwrap().to_string()
        }
    };
    let branch = location(app.tenant_a, "Branch").await;
    let theirs = location(app.tenant_b, "Their branch").await;

    let refused = app
        .admin(
            Method::POST,
            "/api/v1/routing/rules",
            Some(json!({ "name": "theirs", "action": "deny", "location_id": theirs })),
        )
        .await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{}", refused.text);
    assert!(refused.text.contains("not one of this tenant's locations"));
    let bogus = app
        .admin(
            Method::POST,
            "/api/v1/routing/rules",
            Some(json!({ "name": "bogus", "action": "deny", "location_id": "nowhere" })),
        )
        .await;
    assert_eq!(bogus.status, StatusCode::BAD_REQUEST, "{}", bogus.text);

    let at_branch = app
        .admin(
            Method::POST,
            "/api/v1/routing/rules",
            Some(json!({ "name": "branch only", "action": "deny",
                         "destination_pattern": "^\\+44", "location_id": branch })),
        )
        .await;
    assert_eq!(at_branch.status, StatusCode::CREATED, "{}", at_branch.text);
    assert_eq!(at_branch.json["location_id"], branch.as_str());
    let id = at_branch.json["id"].as_str().unwrap().to_string();
    let everywhere = app
        .admin(
            Method::POST,
            "/api/v1/routing/rules",
            Some(json!({ "name": "everywhere", "action": "allow" })),
        )
        .await;
    assert_eq!(
        everywhere.status,
        StatusCode::CREATED,
        "{}",
        everywhere.text
    );
    assert!(everywhere.json["location_id"].is_null());

    let names = |res: &Response| -> Vec<String> {
        res.json
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["name"].as_str().unwrap().to_string())
            .collect()
    };
    let listed = app
        .admin(
            Method::GET,
            &format!("/api/v1/routing/rules?location_id={branch}"),
            None,
        )
        .await;
    assert_eq!(names(&listed), ["branch only"], "{}", listed.text);
    let tenant_wide = app
        .admin(Method::GET, "/api/v1/routing/rules?location_id=none", None)
        .await;
    assert_eq!(names(&tenant_wide), ["everywhere"], "{}", tenant_wide.text);
    let all = app.admin(Method::GET, "/api/v1/routing/rules", None).await;
    assert_eq!(names(&all).len(), 2, "{}", all.text);

    // Its location cannot go from under it.
    let in_use = app
        .admin(Method::DELETE, &format!("/api/v1/locations/{branch}"), None)
        .await;
    assert_eq!(in_use.status, StatusCode::CONFLICT, "{}", in_use.text);
    assert!(
        in_use.text.contains("routing rule branch only"),
        "{}",
        in_use.text
    );

    // Cleared, it is the tenant's again, and the location may go.
    let path = format!("/api/v1/routing/rules/{id}");
    let cleared = app
        .admin(Method::PATCH, &path, Some(json!({ "location_id": "" })))
        .await;
    assert_eq!(cleared.status, StatusCode::OK, "{}", cleared.text);
    assert!(cleared.json["location_id"].is_null(), "{}", cleared.text);
    let moved = app
        .admin(Method::PATCH, &path, Some(json!({ "location_id": theirs })))
        .await;
    assert_eq!(moved.status, StatusCode::BAD_REQUEST, "{}", moved.text);
    let gone = app
        .admin(Method::DELETE, &format!("/api/v1/locations/{branch}"), None)
        .await;
    assert_eq!(gone.status, StatusCode::NO_CONTENT, "{}", gone.text);
    app.finish().await;
}
