//! Numbers, extensions, lines and locations, route by route
//! (docs/DEEP_TESTING.md D3b): what each does, and what it must refuse.

use super::*;
use fcp_core::storage::PhoneNumberStore;
use fcp_core::PhoneNumberId;

impl App {
    async fn create_number(&self, tenant: TenantId, e164: &str, body: Value) -> String {
        let mut body = body;
        body["e164"] = json!(e164);
        let res = self
            .admin_in(tenant, Method::POST, "/api/v1/phone-numbers", Some(body))
            .await;
        assert_eq!(res.status, StatusCode::CREATED, "{}", res.text);
        res.json["id"].as_str().expect("number id").to_string()
    }

    async fn create_location(&self, tenant: TenantId, name: &str) -> String {
        let res = self
            .admin_in(
                tenant,
                Method::POST,
                "/api/v1/locations",
                Some(json!({
                    "name": name,
                    "address_line1": "1 State St",
                    "city": "Albany",
                    "state": "NY",
                    "postal_code": "12207",
                    "country": "US",
                })),
            )
            .await;
        assert_eq!(res.status, StatusCode::CREATED, "{}", res.text);
        res.json["id"].as_str().expect("location id").to_string()
    }
}

fn e911() -> Value {
    json!({
        "address1": "1 State St",
        "city": "Albany",
        "state": "NY",
        "postal_code": "12207",
        "country": "US",
    })
}

/// A number is activated once it has an emergency address, released back
/// to the pool bare, found there by another tenant, bought and deleted.
#[tokio::test]
async fn a_number_is_activated_released_bought_by_another_tenant_and_deleted() {
    let app = require_app!();
    let site = app.create_location(app.tenant_a, "HQ").await;
    let id = app
        .create_number(
            app.tenant_a,
            "+15185550101",
            json!({ "cnam": "ACME", "tags": ["sales"], "location_id": site }),
        )
        .await;
    let path = format!("/api/v1/phone-numbers/{id}");

    // Without an emergency address and callback a number cannot route calls.
    let early = app
        .admin(Method::POST, &format!("{path}/activate"), None)
        .await;
    assert_eq!(early.status, StatusCode::BAD_REQUEST, "{}", early.text);
    assert!(early.text.contains("E911"), "{}", early.text);
    let set = app
        .admin(
            Method::PATCH,
            &path,
            Some(
                json!({ "e911_address": e911(), "e911_callback": "+15185550101",
                         "block_list": ["+15185559999"], "recording_enabled": true }),
            ),
        )
        .await;
    assert_eq!(set.status, StatusCode::OK, "{}", set.text);
    let active = app
        .admin(Method::POST, &format!("{path}/activate"), None)
        .await;
    assert_eq!(active.status, StatusCode::OK, "{}", active.text);
    assert_eq!(active.json["state"], "active");
    assert!(active.json["activated_at"].is_string(), "{}", active.text);
    let again = app
        .admin(Method::POST, &format!("{path}/activate"), None)
        .await;
    assert_eq!(again.status, StatusCode::BAD_REQUEST, "{}", again.text);

    // A number answering a line is not given back while the line stands.
    let user = app.create_user("lena", None).await;
    let line = app
        .admin(
            Method::POST,
            "/api/v1/line-bindings",
            Some(json!({ "phone_number_id": id, "binding_type": "user", "owner_user_id": user })),
        )
        .await;
    assert_eq!(line.status, StatusCode::CREATED, "{}", line.text);
    let line_id = line.json["id"].as_str().unwrap().to_string();
    let bound = app
        .admin(Method::POST, &format!("{path}/release"), None)
        .await;
    assert_eq!(bound.status, StatusCode::CONFLICT, "{}", bound.text);
    let gone = app
        .admin(
            Method::DELETE,
            &format!("/api/v1/line-bindings/{line_id}"),
            None,
        )
        .await;
    assert_eq!(gone.status, StatusCode::NO_CONTENT, "{}", gone.text);

    // Tenant B cannot give back what is A's.
    let not_b = app
        .admin_in(app.tenant_b, Method::POST, &format!("{path}/release"), None)
        .await;
    assert_eq!(not_b.status, StatusCode::NOT_FOUND, "{}", not_b.text);

    let released = app
        .admin(Method::POST, &format!("{path}/release"), None)
        .await;
    assert_eq!(released.status, StatusCode::OK, "{}", released.text);
    let mine = app.admin(Method::GET, &path, None).await;
    assert_eq!(mine.status, StatusCode::NOT_FOUND, "{}", mine.text);

    // In the pool, found by area code and pattern, and nothing of A's on it.
    let found = app
        .admin_in(
            app.tenant_b,
            Method::GET,
            "/api/v1/phone-numbers/available?area_code=518&pattern=*0101",
            None,
        )
        .await;
    assert_eq!(found.status, StatusCode::OK, "{}", found.text);
    let pool = found.json.as_array().unwrap();
    assert_eq!(pool.len(), 1, "{}", found.text);
    let offered = &pool[0];
    assert_eq!(offered["id"], id.as_str());
    assert_eq!(offered["state"], "available");
    for (key, empty) in [
        ("e911_address", Value::Null),
        ("e911_callback", Value::Null),
        ("cnam", Value::Null),
        ("location_id", Value::Null),
        ("activated_at", Value::Null),
        ("tags", json!([])),
        ("recording_enabled", json!(false)),
    ] {
        assert_eq!(offered[key], empty, "{key} went to the pool: {offered}");
    }
    let elsewhere = app
        .admin_in(
            app.tenant_b,
            Method::GET,
            "/api/v1/phone-numbers/available?area_code=212",
            None,
        )
        .await;
    assert_eq!(elsewhere.json, json!([]), "{}", elsewhere.text);

    // B buys it; a second purchase finds it taken, an unknown id nothing.
    let bought = app
        .admin_in(
            app.tenant_b,
            Method::POST,
            &format!("{path}/purchase"),
            None,
        )
        .await;
    assert_eq!(bought.status, StatusCode::OK, "{}", bought.text);
    assert_eq!(bought.json["state"], "assigned");
    assert_eq!(bought.json["tenant_id"], app.tenant_b.to_string());
    assert_eq!(bought.json["block_anonymous"], false);
    let stored = app
        .storage
        .get_phone_number(&id.parse().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert!(stored.block_list.is_empty(), "{:?}", stored.block_list);
    let taken = app
        .admin(Method::POST, &format!("{path}/purchase"), None)
        .await;
    assert_eq!(taken.status, StatusCode::CONFLICT, "{}", taken.text);
    let unknown = app
        .admin(
            Method::POST,
            &format!("/api/v1/phone-numbers/{}/purchase", PhoneNumberId::new()),
            None,
        )
        .await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND, "{}", unknown.text);

    // B deletes it; A cannot, and it is gone.
    let a_delete = app.admin(Method::DELETE, &path, None).await;
    assert_eq!(a_delete.status, StatusCode::NOT_FOUND, "{}", a_delete.text);
    let deleted = app
        .admin_in(app.tenant_b, Method::DELETE, &path, None)
        .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT, "{}", deleted.text);
    let after = app.admin_in(app.tenant_b, Method::GET, &path, None).await;
    assert_eq!(after.status, StatusCode::NOT_FOUND, "{}", after.text);
    app.finish().await;
}

/// An extension is listed, read, changed and deleted by its number, and
/// dials only what is its tenant's.
#[tokio::test]
async fn an_extension_dials_only_its_tenants_things() {
    let app = require_app!();
    let alice = app.create_user("alice", None).await;
    let theirs = app
        .admin_in(
            app.tenant_b,
            Method::POST,
            "/api/v1/users",
            Some(json!({ "username": "mallory", "password": PASSWORD })),
        )
        .await;
    assert_eq!(theirs.status, StatusCode::CREATED, "{}", theirs.text);
    let mallory = theirs.json["id"].as_str().unwrap().to_string();
    let their_number = app
        .create_number(app.tenant_b, "+15185550202", json!({}))
        .await;
    let their_site = app.create_location(app.tenant_b, "B's HQ").await;
    let site = app.create_location(app.tenant_a, "HQ").await;

    // Another tenant's user, number or site is not a target.
    for body in [
        json!({ "number": "2001", "target": { "type": "user", "id": mallory } }),
        json!({ "number": "2001", "target": { "type": "phone_number", "id": their_number } }),
        json!({ "number": "2001", "target": { "type": "user", "id": alice },
                "location_id": their_site }),
    ] {
        let res = app
            .admin(Method::POST, "/api/v1/extensions", Some(body.clone()))
            .await;
        assert_eq!(res.status, StatusCode::BAD_REQUEST, "{body}: {}", res.text);
    }

    let made = app
        .admin(
            Method::POST,
            "/api/v1/extensions",
            Some(
                json!({ "number": "2001", "target": { "type": "user", "id": alice },
                         "description": "front desk", "location_id": site }),
            ),
        )
        .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.text);

    let listed = app.admin(Method::GET, "/api/v1/extensions", None).await;
    assert_eq!(listed.status, StatusCode::OK, "{}", listed.text);
    let numbers: Vec<&str> = listed
        .json
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["number"].as_str().unwrap())
        .collect();
    assert!(numbers.contains(&"2001"), "{}", listed.text);
    let in_b = app
        .admin_in(app.tenant_b, Method::GET, "/api/v1/extensions", None)
        .await;
    assert_eq!(in_b.json, json!([]), "{}", in_b.text);

    let got = app
        .admin(Method::GET, "/api/v1/extensions/2001", None)
        .await;
    assert_eq!(got.status, StatusCode::OK, "{}", got.text);
    assert_eq!(got.json["target"], json!({ "type": "user", "id": alice }));
    assert_eq!(got.json["location_id"], site.as_str());
    let from_b = app
        .admin_in(app.tenant_b, Method::GET, "/api/v1/extensions/2001", None)
        .await;
    assert_eq!(from_b.status, StatusCode::NOT_FOUND, "{}", from_b.text);

    // Changed to point elsewhere: refused for another tenant's user, taken
    // for a number of its own, and its site cleared by an empty value.
    let to_b = app
        .admin(
            Method::PATCH,
            "/api/v1/extensions/2001",
            Some(json!({ "target": { "type": "user", "id": mallory } })),
        )
        .await;
    assert_eq!(to_b.status, StatusCode::BAD_REQUEST, "{}", to_b.text);
    let own_number = app
        .create_number(app.tenant_a, "+15185550203", json!({}))
        .await;
    let changed = app
        .admin(
            Method::PATCH,
            "/api/v1/extensions/2001",
            Some(
                json!({ "target": { "type": "phone_number", "id": own_number },
                         "location_id": "", "enabled": false }),
            ),
        )
        .await;
    assert_eq!(changed.status, StatusCode::OK, "{}", changed.text);
    assert_eq!(changed.json["target"]["type"], "phone_number");
    assert_eq!(changed.json["location_id"], Value::Null);
    assert_eq!(changed.json["enabled"], false);
    assert_eq!(changed.json["description"], "front desk");
    let missing = app
        .admin(
            Method::PATCH,
            "/api/v1/extensions/2999",
            Some(json!({ "enabled": true })),
        )
        .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND, "{}", missing.text);

    let b_delete = app
        .admin_in(
            app.tenant_b,
            Method::DELETE,
            "/api/v1/extensions/2001",
            None,
        )
        .await;
    assert_eq!(b_delete.status, StatusCode::NOT_FOUND, "{}", b_delete.text);
    let deleted = app
        .admin(Method::DELETE, "/api/v1/extensions/2001", None)
        .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT, "{}", deleted.text);
    let after = app
        .admin(Method::GET, "/api/v1/extensions/2001", None)
        .await;
    assert_eq!(after.status, StatusCode::NOT_FOUND, "{}", after.text);
    app.finish().await;
}

/// A line is listed and read in its tenant only, runs only its tenant's
/// flow, and is deleted.
#[tokio::test]
async fn a_line_is_its_tenants_and_runs_only_its_tenants_flow() {
    let app = require_app!();
    let flow = app
        .admin(
            Method::POST,
            "/api/v1/flows",
            Some(hangup_flow("menu-a", "Menu")),
        )
        .await;
    assert_eq!(flow.status, StatusCode::CREATED, "{}", flow.text);
    let their_flow = app
        .admin_in(
            app.tenant_b,
            Method::POST,
            "/api/v1/flows",
            Some(hangup_flow("menu-b", "Their menu")),
        )
        .await;
    assert_eq!(
        their_flow.status,
        StatusCode::CREATED,
        "{}",
        their_flow.text
    );
    let number = app
        .create_number(app.tenant_a, "+15185550301", json!({}))
        .await;

    let refused = app
        .admin(
            Method::POST,
            "/api/v1/line-bindings",
            Some(json!({ "phone_number_id": number, "binding_type": "ivr", "flow_id": "menu-b" })),
        )
        .await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{}", refused.text);
    let unknown = app
        .admin(
            Method::POST,
            "/api/v1/line-bindings",
            Some(json!({ "phone_number_id": number, "binding_type": "ivr", "flow_id": "nope" })),
        )
        .await;
    assert_eq!(unknown.status, StatusCode::BAD_REQUEST, "{}", unknown.text);

    let made = app
        .admin(
            Method::POST,
            "/api/v1/line-bindings",
            Some(json!({ "phone_number_id": number, "binding_type": "ivr", "flow_id": "menu-a" })),
        )
        .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.text);
    let id = made.json["id"].as_str().unwrap().to_string();
    let path = format!("/api/v1/line-bindings/{id}");
    assert_eq!(made.json["flow_id"], "menu-a");

    let to_theirs = app
        .admin(Method::PATCH, &path, Some(json!({ "flow_id": "menu-b" })))
        .await;
    assert_eq!(
        to_theirs.status,
        StatusCode::BAD_REQUEST,
        "{}",
        to_theirs.text
    );
    let cleared = app
        .admin(Method::PATCH, &path, Some(json!({ "flow_id": "" })))
        .await;
    assert_eq!(cleared.status, StatusCode::OK, "{}", cleared.text);
    assert_eq!(cleared.json["flow_id"], Value::Null, "{}", cleared.text);

    let listed = app.admin(Method::GET, "/api/v1/line-bindings", None).await;
    assert_eq!(listed.status, StatusCode::OK, "{}", listed.text);
    let ids: Vec<&str> = listed
        .json
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec![id.as_str()], "{}", listed.text);
    let in_b = app
        .admin_in(app.tenant_b, Method::GET, "/api/v1/line-bindings", None)
        .await;
    assert_eq!(in_b.json, json!([]), "{}", in_b.text);

    let got = app.admin(Method::GET, &path, None).await;
    assert_eq!(got.status, StatusCode::OK, "{}", got.text);
    assert_eq!(got.json["phone_number_id"], number.as_str());
    let from_b = app.admin_in(app.tenant_b, Method::GET, &path, None).await;
    assert_eq!(from_b.status, StatusCode::NOT_FOUND, "{}", from_b.text);
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

/// A site is listed in its tenant, and is not deleted while anything is
/// placed there: each would be left at no address.
#[tokio::test]
async fn a_location_in_use_is_not_deleted() {
    let app = require_app!();
    let site = app.create_location(app.tenant_a, "HQ").await;
    let other = app.create_location(app.tenant_a, "Annex").await;
    let path = format!("/api/v1/locations/{site}");

    let listed = app.admin(Method::GET, "/api/v1/locations", None).await;
    assert_eq!(listed.status, StatusCode::OK, "{}", listed.text);
    let names: Vec<&str> = listed
        .json
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["Annex", "HQ"], "{}", listed.text);
    let in_b = app
        .admin_in(app.tenant_b, Method::GET, "/api/v1/locations", None)
        .await;
    assert_eq!(in_b.json, json!([]), "{}", in_b.text);

    // Each kind of thing placed at the site holds it, until moved.
    let number = app
        .create_number(app.tenant_a, "+15185550401", json!({ "location_id": site }))
        .await;
    let refused = app.admin(Method::DELETE, &path, None).await;
    assert_eq!(refused.status, StatusCode::CONFLICT, "{}", refused.text);
    assert!(refused.text.contains("+15185550401"), "{}", refused.text);
    let moved = app
        .admin(
            Method::PATCH,
            &format!("/api/v1/phone-numbers/{number}"),
            Some(json!({ "location_id": other })),
        )
        .await;
    assert_eq!(moved.status, StatusCode::OK, "{}", moved.text);

    let network = app
        .admin(
            Method::POST,
            "/api/v1/emergency/networks",
            Some(json!({ "cidr": "10.1.0.0/16", "location_id": site })),
        )
        .await;
    assert_eq!(network.status, StatusCode::CREATED, "{}", network.text);
    let refused = app.admin(Method::DELETE, &path, None).await;
    assert_eq!(refused.status, StatusCode::CONFLICT, "{}", refused.text);
    assert!(refused.text.contains("10.1.0.0/16"), "{}", refused.text);
    let network_id = network.json["id"].as_str().unwrap();
    let unmapped = app
        .admin(
            Method::DELETE,
            &format!("/api/v1/emergency/networks/{network_id}"),
            None,
        )
        .await;
    assert_eq!(unmapped.status, StatusCode::NO_CONTENT, "{}", unmapped.text);

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
