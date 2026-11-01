//! Queues, groups and presence, route by route (docs/DEEP_TESTING.md D3b).

use super::*;
use fcp_presence::{PresenceAuthPolicy, Watcher};

/// Every queue's stats at once, in the tenant that has the queue; an
/// agent set away and back by an administrator.
#[tokio::test]
async fn queue_stats_for_all_and_an_agent_set_away_and_back() {
    let app = require_app!();
    let login = app
        .admin(
            Method::POST,
            "/api/v1/queues/support/agents",
            Some(json!({ "agent_id": "agent-7", "extension": "1007" })),
        )
        .await;
    assert_eq!(login.status, StatusCode::CREATED, "{}", login.text);

    let all = app.admin(Method::GET, "/api/v1/queues/stats", None).await;
    assert_eq!(all.status, StatusCode::OK, "{}", all.text);
    let rows = all.json.as_array().unwrap();
    assert_eq!(rows.len(), 1, "{}", all.text);
    assert_eq!(rows[0]["queue_name"], "support");
    assert_eq!(rows[0]["agents_total"], 1);
    assert_eq!(rows[0]["agents_available"], 1);
    let in_b = app
        .admin_in(app.tenant_b, Method::GET, "/api/v1/queues/stats", None)
        .await;
    assert_eq!(in_b.status, StatusCode::OK, "{}", in_b.text);
    assert_eq!(in_b.json, json!([]), "{}", in_b.text);

    let agent = "/api/v1/queues/support/agents/agent-7";
    let away = app
        .admin(Method::PATCH, agent, Some(json!({ "state": "away" })))
        .await;
    assert_eq!(away.status, StatusCode::OK, "{}", away.text);
    assert!(
        away.json["state"]
            .as_str()
            .unwrap()
            .eq_ignore_ascii_case("away"),
        "{}",
        away.text
    );
    let stats = app.admin(Method::GET, "/api/v1/queues/stats", None).await;
    assert_eq!(stats.json[0]["agents_available"], 0, "{}", stats.text);

    let back = app
        .admin(Method::PATCH, agent, Some(json!({ "state": "available" })))
        .await;
    assert_eq!(back.status, StatusCode::OK, "{}", back.text);
    let stats = app.admin(Method::GET, "/api/v1/queues/stats", None).await;
    assert_eq!(stats.json[0]["agents_available"], 1, "{}", stats.text);

    // Only the two states an administrator sets; not another tenant's
    // agent; not an agent the queue does not have.
    let busy = app
        .admin(Method::PATCH, agent, Some(json!({ "state": "on_call" })))
        .await;
    assert_eq!(busy.status, StatusCode::BAD_REQUEST, "{}", busy.text);
    let from_b = app
        .admin_in(
            app.tenant_b,
            Method::PATCH,
            agent,
            Some(json!({ "state": "away" })),
        )
        .await;
    assert_eq!(from_b.status, StatusCode::NOT_FOUND, "{}", from_b.text);
    let nobody = app
        .admin(
            Method::PATCH,
            "/api/v1/queues/support/agents/agent-404",
            Some(json!({ "state": "away" })),
        )
        .await;
    assert_eq!(nobody.status, StatusCode::NOT_FOUND, "{}", nobody.text);
    app.finish().await;
}

/// A queue's configuration is changed in its tenant, checked as a new
/// one is: a name, an extension, and a strategy a node runs.
#[tokio::test]
async fn a_queue_config_is_changed_and_checked() {
    let app = require_app!();
    let unknown = app
        .admin(
            Method::POST,
            "/api/v1/queue-configs",
            Some(json!({ "name": "sales", "extension": "7200", "strategy": "ring_all" })),
        )
        .await;
    assert_eq!(unknown.status, StatusCode::BAD_REQUEST, "{}", unknown.text);
    assert!(unknown.text.contains("ring_all"), "{}", unknown.text);

    let made = app
        .admin(
            Method::POST,
            "/api/v1/queue-configs",
            Some(json!({ "name": "sales", "extension": "7200" })),
        )
        .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.text);
    assert_eq!(made.json["strategy"], "round_robin");
    let path = format!(
        "/api/v1/queue-configs/{}",
        made.json["id"].as_str().unwrap()
    );

    for body in [
        json!({ "name": " " }),
        json!({ "extension": "" }),
        json!({ "strategy": "ring_all" }),
    ] {
        let res = app.admin(Method::PATCH, &path, Some(body.clone())).await;
        assert_eq!(res.status, StatusCode::BAD_REQUEST, "{body}: {}", res.text);
    }
    let changed = app
        .admin(
            Method::PATCH,
            &path,
            Some(
                json!({ "strategy": "skills_based", "required_skills": ["spanish"],
                         "max_wait_time_secs": 90, "enabled": false }),
            ),
        )
        .await;
    assert_eq!(changed.status, StatusCode::OK, "{}", changed.text);
    assert_eq!(changed.json["strategy"], "skills_based");
    assert_eq!(changed.json["required_skills"], json!(["spanish"]));
    assert_eq!(changed.json["max_wait_time_secs"], 90);
    assert_eq!(changed.json["enabled"], false);
    assert_eq!(changed.json["name"], "sales", "left alone");
    let read_back = app.admin(Method::GET, &path, None).await;
    assert_eq!(
        read_back.json["strategy"], "skills_based",
        "{}",
        read_back.text
    );

    // A name another tenant's queue has is this tenant's to take: a queue
    // is its name within its tenant (docs/TENANT_SCOPE.md §4b).
    let theirs = app
        .admin_in(
            app.tenant_b,
            Method::POST,
            "/api/v1/queue-configs",
            Some(json!({ "name": "billing", "extension": "7300" })),
        )
        .await;
    assert_eq!(theirs.status, StatusCode::CREATED, "{}", theirs.text);
    let same_name = app
        .admin(Method::PATCH, &path, Some(json!({ "name": "billing" })))
        .await;
    assert_eq!(same_name.status, StatusCode::OK, "{}", same_name.text);
    assert_eq!(same_name.json["name"], "billing");

    let from_b = app
        .admin_in(
            app.tenant_b,
            Method::PATCH,
            &path,
            Some(json!({ "enabled": true })),
        )
        .await;
    assert_eq!(from_b.status, StatusCode::NOT_FOUND, "{}", from_b.text);
    app.finish().await;
}

/// A group is replaced by a PUT: renamed, its role changed or taken away,
/// a platform role refused, a name the tenant has refused.
#[tokio::test]
async fn a_group_is_replaced_and_checked() {
    let app = require_app!();
    let make = |name: &str| {
        let app = &app;
        let name = name.to_string();
        async move {
            let res = app
                .admin(
                    Method::POST,
                    "/api/v1/groups",
                    Some(json!({ "name": name })),
                )
                .await;
            assert_eq!(res.status, StatusCode::CREATED, "{}", res.text);
            res.json["id"].as_str().unwrap().to_string()
        }
    };
    let id = make("Helpdesk").await;
    make("Sales").await;
    let path = format!("/api/v1/groups/{id}");

    let replaced = app
        .admin(
            Method::PUT,
            &path,
            Some(json!({ "name": "Service desk", "description": "first line",
                         "role": "user_admin", "idp_group": "sd-staff" })),
        )
        .await;
    assert_eq!(replaced.status, StatusCode::OK, "{}", replaced.text);
    assert_eq!(replaced.json["name"], "Service desk");
    assert_eq!(replaced.json["role"], "user_admin");
    assert_eq!(replaced.json["idp_group"], "sd-staff");
    let read_back = app.admin(Method::GET, &path, None).await;
    assert_eq!(
        read_back.json["description"], "first line",
        "{}",
        read_back.text
    );

    // A PUT is the whole group: what it leaves out is gone.
    let bare = app
        .admin(Method::PUT, &path, Some(json!({ "name": "Service desk" })))
        .await;
    assert_eq!(bare.status, StatusCode::OK, "{}", bare.text);
    assert_eq!(bare.json["role"], Value::Null, "{}", bare.text);
    assert_eq!(bare.json["description"], Value::Null, "{}", bare.text);

    for (body, status) in [
        (
            json!({ "name": "Root", "role": "super_admin" }),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({ "name": "x", "role": "wizard" }),
            StatusCode::BAD_REQUEST,
        ),
        (json!({ "name": "" }), StatusCode::BAD_REQUEST),
        (json!({ "name": "sales" }), StatusCode::CONFLICT),
    ] {
        let res = app.admin(Method::PUT, &path, Some(body.clone())).await;
        assert_eq!(res.status, status, "{body}: {}", res.text);
    }
    let from_b = app
        .admin_in(
            app.tenant_b,
            Method::PUT,
            &path,
            Some(json!({ "name": "Ours" })),
        )
        .await;
    assert_eq!(from_b.status, StatusCode::NOT_FOUND, "{}", from_b.text);
    app.finish().await;
}

/// Pres-rules XML sets a presentity's policy, read back as JSON and XML;
/// XML that is not pres-rules is refused.
#[tokio::test]
async fn pres_rules_set_the_policy() {
    let app = require_app!();
    let aor = format!("carol@{REALM}");
    let path = format!("/api/v1/presence/{aor}");
    let policy = PresenceAuthPolicy {
        default_allow: false,
        allow_list: [format!("dave@{REALM}")].into_iter().collect(),
        block_list: [format!("eve@{REALM}")].into_iter().collect(),
        ..Default::default()
    };
    let xml = fcp_presence::xcap::generate_pres_rules(&policy);

    let set = app
        .send_raw(
            ADMIN_TOKEN,
            Method::PUT,
            &format!("{path}/pres-rules"),
            "application/auth-policy+xml",
            xml.into_bytes(),
        )
        .await;
    assert_eq!(set.status, StatusCode::OK, "{}", set.text);
    assert_eq!(
        set.header("content-type"),
        Some("application/auth-policy+xml")
    );
    assert!(set.text.contains(&format!("dave@{REALM}")), "{}", set.text);

    let as_json = app
        .admin(Method::GET, &format!("{path}/policy"), None)
        .await;
    assert_eq!(as_json.status, StatusCode::OK, "{}", as_json.text);
    assert_eq!(as_json.json["default_allow"], false, "{}", as_json.text);
    assert_eq!(as_json.json["allow_list"], json!([format!("dave@{REALM}")]));
    assert_eq!(as_json.json["block_list"], json!([format!("eve@{REALM}")]));

    // Another tenant's presentity of that address keeps its own policy.
    let theirs = app
        .admin_in(
            app.tenant_b,
            Method::GET,
            &format!("{path}/pres-rules"),
            None,
        )
        .await;
    assert_eq!(theirs.status, StatusCode::OK, "{}", theirs.text);
    assert!(!theirs.text.contains("dave@"), "{}", theirs.text);

    let garbage = app
        .send_raw(
            ADMIN_TOKEN,
            Method::PUT,
            &format!("{path}/pres-rules"),
            "application/auth-policy+xml",
            b"<not-closed".to_vec(),
        )
        .await;
    assert_eq!(garbage.status, StatusCode::BAD_REQUEST, "{}", garbage.text);
    app.finish().await;
}

/// An administrator ends a watcher's subscription: this tenant's, never
/// another tenant's watching the same address.
#[tokio::test]
async fn a_watcher_is_ended_in_its_tenant_only() {
    let app = require_app!();
    let pm = app
        .state
        .presence_manager
        .clone()
        .expect("presence manager");
    let aor = format!("frank@{REALM}");
    let watcher = |tenant: TenantId, sub: &str| Watcher {
        watcher_aor: format!("watcher-{sub}@{REALM}"),
        presentity_aor: aor.clone(),
        subscription_id: sub.to_string(),
        expires_at: std::time::Instant::now() + Duration::from_secs(3600),
        tenant_id: tenant,
        contact_uri: "sip:watcher@192.0.2.10:5060".to_string(),
        event_package: "presence".to_string(),
        sip_call_id: None,
        sip_from_tag: None,
        sip_to_tag: None,
    };
    // B's watcher first, while no tenant has the presentity; then A's.
    pm.add_watcher(watcher(app.tenant_b, "sub-b"))
        .await
        .unwrap();
    let path = format!("/api/v1/presence/{aor}");
    let set = app
        .admin(Method::PUT, &path, Some(json!({ "state": "open" })))
        .await;
    assert_eq!(set.status, StatusCode::OK, "{}", set.text);
    pm.add_watcher(watcher(app.tenant_a, "sub-a"))
        .await
        .unwrap();

    let not_ours = app
        .admin(Method::DELETE, &format!("{path}/watchers/sub-b"), None)
        .await;
    assert_eq!(not_ours.status, StatusCode::NOT_FOUND, "{}", not_ours.text);
    assert_eq!(pm.get_watchers_for_tenant(&aor, &app.tenant_b).len(), 1);

    let ended = app
        .admin(Method::DELETE, &format!("{path}/watchers/sub-a"), None)
        .await;
    assert_eq!(ended.status, StatusCode::NO_CONTENT, "{}", ended.text);
    let watchers = app
        .admin(Method::GET, &format!("{path}/watchers"), None)
        .await;
    assert_eq!(watchers.json, json!([]), "{}", watchers.text);
    let again = app
        .admin(Method::DELETE, &format!("{path}/watchers/sub-a"), None)
        .await;
    assert_eq!(again.status, StatusCode::NOT_FOUND, "{}", again.text);
    let no_presentity = app
        .admin(
            Method::DELETE,
            &format!("/api/v1/presence/nobody@{REALM}/watchers/sub-a"),
            None,
        )
        .await;
    assert_eq!(
        no_presentity.status,
        StatusCode::NOT_FOUND,
        "{}",
        no_presentity.text
    );
    app.finish().await;
}
