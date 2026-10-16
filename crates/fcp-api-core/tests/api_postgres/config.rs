//! The services' configuration and phone templates, route by route
//! (docs/DEEP_TESTING.md D3b).

use super::*;

/// The platform's administrator writes a section, reads it back by
/// service and section, sees each change in the audit log, checks a value
/// without saving it and deletes the section. A tenant's administrator
/// reads none of it: the nodes keep their own credentials there.
#[tokio::test]
async fn the_services_configuration_is_the_platforms() {
    let app = require_app!();
    let section = "/api/v1/config/media/auth";
    let first = app
        .admin(
            Method::PUT,
            section,
            Some(json!({ "value": { "tokens": ["node-secret-1"] }, "updated_by": "ops" })),
        )
        .await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text);
    assert_eq!(first.json["tenant_id"], "_system");
    let v1 = first.json["version"].as_i64().unwrap();
    let second = app
        .admin(
            Method::PUT,
            section,
            Some(json!({ "value": { "tokens": ["node-secret-2"] } })),
        )
        .await;
    assert_eq!(second.status, StatusCode::OK, "{}", second.text);
    assert!(
        second.json["version"].as_i64().unwrap() > v1,
        "{}",
        second.text
    );
    let other = app
        .admin(
            Method::PUT,
            "/api/v1/config/media/recording",
            Some(json!({ "value": { "enabled": true } })),
        )
        .await;
    assert_eq!(other.status, StatusCode::OK, "{}", other.text);
    let not_object = app
        .admin(Method::PUT, section, Some(json!({ "value": [1, 2] })))
        .await;
    assert_eq!(
        not_object.status,
        StatusCode::BAD_REQUEST,
        "{}",
        not_object.text
    );

    let all = app.admin(Method::GET, "/api/v1/config", None).await;
    assert_eq!(all.status, StatusCode::OK, "{}", all.text);
    let names: Vec<String> = all
        .json
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            format!(
                "{}/{}",
                e["service"].as_str().unwrap(),
                e["section"].as_str().unwrap()
            )
        })
        .collect();
    assert_eq!(names, vec!["media/auth", "media/recording"], "{}", all.text);
    let sbc_only = app
        .admin(Method::GET, "/api/v1/config?service=sbc", None)
        .await;
    assert_eq!(sbc_only.json, json!([]), "{}", sbc_only.text);

    let service = app.admin(Method::GET, "/api/v1/config/media", None).await;
    assert_eq!(service.status, StatusCode::OK, "{}", service.text);
    assert_eq!(
        service.json.as_array().unwrap().len(),
        2,
        "{}",
        service.text
    );

    let got = app.admin(Method::GET, section, None).await;
    assert_eq!(got.status, StatusCode::OK, "{}", got.text);
    assert_eq!(got.json["value"], json!({ "tokens": ["node-secret-2"] }));

    // Every change, newest first, with what it replaced.
    let audit = app
        .admin(
            Method::GET,
            "/api/v1/config/audit?service=media&section=auth",
            None,
        )
        .await;
    assert_eq!(audit.status, StatusCode::OK, "{}", audit.text);
    let changes = audit.json.as_array().unwrap();
    assert_eq!(changes.len(), 2, "{}", audit.text);
    assert_eq!(
        changes[0]["new_value"],
        json!({ "tokens": ["node-secret-2"] })
    );
    assert_eq!(
        changes[0]["old_value"],
        json!({ "tokens": ["node-secret-1"] })
    );
    assert_eq!(changes[1]["old_value"], Value::Null, "{}", audit.text);
    assert_eq!(changes[1]["changed_by"], "ops");

    // Checked, not saved.
    for (value, valid) in [
        (json!({ "tokens": [] }), true),
        (json!({}), false),
        (json!("tokens"), false),
    ] {
        let res = app
            .admin(
                Method::POST,
                "/api/v1/config/media/auth/validate",
                Some(json!({ "value": value })),
            )
            .await;
        assert_eq!(res.status, StatusCode::OK, "{}", res.text);
        assert_eq!(res.json["valid"], valid, "{value}: {}", res.text);
        assert_eq!(
            res.json["errors"].as_array().unwrap().is_empty(),
            valid,
            "{}",
            res.text
        );
    }
    let unchanged = app.admin(Method::GET, section, None).await;
    assert_eq!(
        unchanged.json["value"], got.json["value"],
        "{}",
        unchanged.text
    );

    // A tenant's administrator, a telecom administrator and a read-only
    // user read nothing of it, and do not reach another tenant's either.
    for (name, role) in [
        ("ta", "tenant_admin"),
        ("tel", "telecom_admin"),
        ("ro", "read_only"),
    ] {
        app.create_user_with_role(name, role).await;
        let jwt = app.login(name, PASSWORD).await.json["token"]
            .as_str()
            .unwrap()
            .to_string();
        for path in [
            "/api/v1/config".to_string(),
            section.to_string(),
            "/api/v1/config/media".to_string(),
            "/api/v1/config/audit?service=media".to_string(),
            format!("/api/v1/config/media/auth?tenant_id={}", app.tenant_b),
        ] {
            let res = app.send_as(&jwt, Method::GET, &path, None).await;
            assert_eq!(
                res.status,
                StatusCode::FORBIDDEN,
                "{role} {path}: {}",
                res.text
            );
            assert!(!res.text.contains("node-secret"), "{}", res.text);
        }
    }

    let deleted = app.admin(Method::DELETE, section, None).await;
    assert_eq!(deleted.status, StatusCode::OK, "{}", deleted.text);
    assert_eq!(deleted.json["deleted"], true);
    let again = app.admin(Method::DELETE, section, None).await;
    assert_eq!(again.json["deleted"], false, "{}", again.text);
    let gone = app.admin(Method::GET, section, None).await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND, "{}", gone.text);
    app.finish().await;
}

/// A tenant's phone templates: listed and read in the tenant, changed
/// with their body checked, and not deleted while a phone names one.
#[tokio::test]
async fn a_phone_template_is_changed_and_not_deleted_from_under_a_phone() {
    let app = require_app!();
    let template = app.create_template(app.tenant_a, "line=${line}\n").await;
    let theirs = app.create_template(app.tenant_b, "theirs\n").await;
    let path = format!("/api/v1/config-templates/{template}");

    let listed = app
        .admin(Method::GET, "/api/v1/config-templates", None)
        .await;
    assert_eq!(listed.status, StatusCode::OK, "{}", listed.text);
    let ids: Vec<&str> = listed
        .json
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec![template.as_str()], "{}", listed.text);

    let got = app.admin(Method::GET, &path, None).await;
    assert_eq!(got.status, StatusCode::OK, "{}", got.text);
    assert_eq!(got.json["vendor"], "yealink");
    assert_eq!(got.json["template_body"], "line=${line}\n");
    let not_ours = app
        .admin(
            Method::GET,
            &format!("/api/v1/config-templates/{theirs}"),
            None,
        )
        .await;
    assert_eq!(not_ours.status, StatusCode::NOT_FOUND, "{}", not_ours.text);

    // A body a phone could not be given, and a vendor there is none of,
    // are refused; the rest is kept.
    let broken = app
        .admin(
            Method::PATCH,
            &path,
            Some(json!({ "template_body": "{% if x %}never closed" })),
        )
        .await;
    assert_eq!(broken.status, StatusCode::BAD_REQUEST, "{}", broken.text);
    assert!(broken.text.contains("template_body"), "{}", broken.text);
    let no_vendor = app
        .admin(Method::PATCH, &path, Some(json!({ "vendor": "acme" })))
        .await;
    assert_eq!(
        no_vendor.status,
        StatusCode::BAD_REQUEST,
        "{}",
        no_vendor.text
    );
    let changed = app
        .admin(
            Method::PATCH,
            &path,
            Some(json!({ "name": "Yealink T5x", "model_pattern": "T5*",
                         "template_body": "{{ device_name }}\n", "vendor": "polycom" })),
        )
        .await;
    assert_eq!(changed.status, StatusCode::OK, "{}", changed.text);
    assert_eq!(changed.json["name"], "Yealink T5x");
    assert_eq!(changed.json["model_pattern"], "T5*");
    assert_eq!(changed.json["vendor"], "polycom");
    let any_model = app
        .admin(Method::PATCH, &path, Some(json!({ "model_pattern": "" })))
        .await;
    assert_eq!(
        any_model.json["model_pattern"],
        Value::Null,
        "{}",
        any_model.text
    );
    assert_eq!(any_model.json["template_body"], "{{ device_name }}\n");
    let b_patch = app
        .admin_in(
            app.tenant_b,
            Method::PATCH,
            &path,
            Some(json!({ "name": "mine" })),
        )
        .await;
    assert_eq!(b_patch.status, StatusCode::NOT_FOUND, "{}", b_patch.text);

    // A phone that names it holds it.
    let (device, _) = app
        .create_phone("Front desk", "AA:BB:CC:DD:EE:51", None)
        .await;
    let named = app
        .patch_device(&device, json!({ "config_template_id": template }))
        .await;
    assert_eq!(named.status, StatusCode::OK, "{}", named.text);
    let held = app.admin(Method::DELETE, &path, None).await;
    assert_eq!(held.status, StatusCode::CONFLICT, "{}", held.text);
    assert!(held.text.contains("Front desk"), "{}", held.text);
    let b_delete = app
        .admin_in(app.tenant_b, Method::DELETE, &path, None)
        .await;
    assert_eq!(b_delete.status, StatusCode::NOT_FOUND, "{}", b_delete.text);

    let other = app.create_template(app.tenant_a, "other\n").await;
    let moved = app
        .patch_device(&device, json!({ "config_template_id": other }))
        .await;
    assert_eq!(moved.status, StatusCode::OK, "{}", moved.text);
    let deleted = app.admin(Method::DELETE, &path, None).await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT, "{}", deleted.text);
    let after = app.admin(Method::GET, &path, None).await;
    assert_eq!(after.status, StatusCode::NOT_FOUND, "{}", after.text);
    app.finish().await;
}
