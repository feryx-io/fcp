//! Live call control goes to the call manager holding the call
//! (`docs/DEVELOPER_SURFACE.md` phase 8): `/api/v1/calls/:id/{hold,…}` is
//! put to the node `[call_manager] url` names, with its token, the
//! caller's tenant and the request's fields, and the node's answer comes
//! back — its errors in the API's one shape. So are a call's details, its
//! hangup by `DELETE` and its recording. What the node then does is
//! proved against the binary in `fcp-call-manager/tests/calling.rs` and
//! `calls.rs`.
//! Skipped without `FCP_TEST_DATABASE_URL`.

use axum::extract::{Path, Query};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use fcp_testkit::smoke::{self, free_port, loopback};
use fcp_testkit::{Service, TestDatabase};
use std::sync::{Arc, Mutex};

const BIN: &str = env!("CARGO_BIN_EXE_fcp-admin");
/// The one call the stand-in holds.
const CALL: &str = "call-11111111-2222-3333-4444-555555555555";

#[derive(Debug, Clone)]
struct Asked {
    path: String,
    authorization: Option<String>,
    body: serde_json::Value,
}

/// A call manager holding [`CALL`]: it answers each control as the node
/// does, a call it does not hold with 404 "Call not found", and a hold of
/// the callee with 409.
async fn stand_in(asked: Arc<Mutex<Vec<Asked>>>) -> u16 {
    let port = free_port();
    let shown = Arc::clone(&asked);
    let recorded = Arc::clone(&asked);
    let app = Router::new()
        .route(
            "/calls/:id",
            get(
                move |Path(id): Path<String>,
                      Query(query): Query<std::collections::HashMap<String, String>>| {
                    let asked = Arc::clone(&shown);
                    async move {
                        asked.lock().unwrap().push(Asked {
                            path: format!("/calls/{id}"),
                            authorization: None,
                            body: serde_json::json!({ "tenant_id": query.get("tenant_id") }),
                        });
                        if id != CALL {
                            return (
                                StatusCode::NOT_FOUND,
                                Json(serde_json::json!({ "error": format!("Call not found: {id}") })),
                            );
                        }
                        (
                            StatusCode::OK,
                            Json(serde_json::json!({
                                "id": id, "state": "Established", "from_uri": "sip:a@x",
                                "to_uri": "sip:b@x", "from_display_name": null,
                                "to_display_name": null,
                                "created_at": "2026-10-16T09:00:00Z", "established_at": null,
                                "ended_at": null, "duration_seconds": null,
                                "media_anchored": true, "failure_reason": null,
                            })),
                        )
                    }
                },
            ),
        )
        .route(
            "/calls/:id/recording/:what",
            post(
                move |Path((id, what)): Path<(String, String)>,
                      Json(body): Json<serde_json::Value>| {
                    let asked = Arc::clone(&recorded);
                    async move {
                        asked.lock().unwrap().push(Asked {
                            path: format!("/calls/{id}/recording/{what}"),
                            authorization: None,
                            body,
                        });
                        let status = if what == "start" { "recording" } else { "stopped" };
                        (
                            StatusCode::OK,
                            Json(serde_json::json!({ "call_id": id, "status": status })),
                        )
                    }
                },
            ),
        )
        .route(
        "/calls/:id/:action",
        post(
            move |Path((id, action)): Path<(String, String)>,
                  headers: HeaderMap,
                  Json(body): Json<serde_json::Value>| {
                let asked = Arc::clone(&asked);
                async move {
                    asked.lock().unwrap().push(Asked {
                        path: format!("/calls/{id}/{action}"),
                        authorization: headers
                            .get("authorization")
                            .and_then(|v| v.to_str().ok())
                            .map(str::to_string),
                        body: body.clone(),
                    });
                    if id != CALL {
                        return (
                            StatusCode::NOT_FOUND,
                            Json(serde_json::json!({ "error": format!("Call not found: {id}") })),
                        );
                    }
                    if action == "hold" && body["party"] == "callee" {
                        return (
                            StatusCode::CONFLICT,
                            Json(serde_json::json!({
                                "error": "the other party is on hold; resume the call first"
                            })),
                        );
                    }
                    let status = match action.as_str() {
                        "transfer" | "redirect" => StatusCode::ACCEPTED,
                        _ => StatusCode::OK,
                    };
                    (
                        status,
                        Json(serde_json::json!({ "call_id": id, "status": action })),
                    )
                }
            },
        ),
    );
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .expect("bind the stand-in");
    tokio::spawn(async move { axum::serve(listener, app).await });
    port
}

#[tokio::test]
async fn call_control_is_asked_of_the_call_manager_holding_the_call() {
    let Some(db) = TestDatabase::create().await else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping");
        return;
    };
    let asked = Arc::new(Mutex::new(Vec::new()));
    let node = stand_in(Arc::clone(&asked)).await;
    let dir = smoke::temp_dir();
    let http = free_port();
    let config = format!(
        r#"prompts_dir = "{dir}/prompts"

[service]
node_id = "control-admin"

[api]
bind_address = "127.0.0.1:{http}"
auth_enabled = false

[storage]
database_url = "{db}"

[call_manager]
url = "http://127.0.0.1:{node}"
auth_token = "cm-secret"

[logging]
level = "info"
format = "compact"
"#,
        db = db.url(),
        dir = smoke::toml_path(dir.path()),
    );
    let service = Service::start("admin", BIN, &config, &loopback(http, "/api/v1/health")).await;
    let client = reqwest::Client::new();
    let post = |action: &str, call: &str, body: Option<serde_json::Value>| {
        let mut request = client.post(loopback(http, &format!("/api/v1/calls/{call}/{action}")));
        if let Some(body) = body {
            request = request.json(&body);
        }
        async move {
            let response = request.send().await.expect("the admin answers");
            let status = response.status().as_u16();
            let text = response.text().await.unwrap();
            (status, text)
        }
    };

    // Every control reaches the node, with its fields, the caller's tenant
    // and the node's token; the node's answer comes back.
    let controls: [(&str, Option<serde_json::Value>, u16); 7] = [
        ("hold", Some(serde_json::json!({"party": "caller"})), 200),
        ("resume", None, 200),
        (
            "transfer",
            Some(serde_json::json!({"target": "1003", "party": "caller"})),
            202,
        ),
        (
            "redirect",
            Some(serde_json::json!({"flow_id": "survey"})),
            202,
        ),
        (
            "play",
            Some(serde_json::json!({"prompt_id": "prompt-1", "to": "callee"})),
            200,
        ),
        (
            "say",
            Some(serde_json::json!({"text": "One moment", "language": "en-GB"})),
            200,
        ),
        ("hangup", None, 200),
    ];
    for (action, body, expected) in &controls {
        let (status, text) = post(action, CALL, body.clone()).await;
        assert_eq!(status, *expected, "{action}: {text}");
        let answer: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(answer["status"], *action, "{text}");
        let seen = asked
            .lock()
            .unwrap()
            .last()
            .cloned()
            .expect("the node was asked");
        assert_eq!(seen.path, format!("/calls/{CALL}/{action}"));
        assert_eq!(seen.authorization.as_deref(), Some("Bearer cm-secret"));
        assert!(
            seen.body["tenant_id"]
                .as_str()
                .is_some_and(|t| !t.is_empty()),
            "{action}: {:?}",
            seen.body
        );
        if let Some(body) = body {
            for (field, value) in body.as_object().unwrap() {
                assert_eq!(&seen.body[field], value, "{action}: {field}");
            }
        }
    }

    // The node's refusal, in the API's one error shape.
    let (status, text) = post("hold", CALL, Some(serde_json::json!({"party": "callee"}))).await;
    assert_eq!(status, 409, "{text}");
    let error: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(error["code"], "conflict", "{text}");
    assert!(
        error["details"].as_str().unwrap().contains("on hold"),
        "the node's reason: {text}"
    );

    // A call no node holds.
    let (status, text) = post("hangup", "call-00000000-0000-0000-0000-000000000000", None).await;
    assert_eq!(status, 404, "{text}");

    // A call's details, its recording and its hangup by DELETE go the same
    // way, for the caller's tenant.
    let call_url = loopback(http, &format!("/api/v1/calls/{CALL}"));
    let shown = client.get(&call_url).send().await.unwrap();
    assert_eq!(shown.status(), 200);
    let shown: serde_json::Value = shown.json().await.unwrap();
    assert_eq!(shown["id"], CALL, "{shown}");
    let tenant = asked.lock().unwrap().last().unwrap().body["tenant_id"].clone();
    assert!(tenant.as_str().is_some_and(|t| !t.is_empty()), "{tenant}");
    for (what, body, status) in [
        (
            "start",
            Some(serde_json::json!({"path": "calls/x.wav"})),
            "recording",
        ),
        ("stop", None, "stopped"),
    ] {
        let (code, text) = post(&format!("recording/{what}"), CALL, body.clone()).await;
        assert_eq!(code, 200, "{what}: {text}");
        assert!(text.contains(status), "{text}");
        let seen = asked.lock().unwrap().last().cloned().unwrap();
        assert_eq!(seen.path, format!("/calls/{CALL}/recording/{what}"));
        let bare = |v: &serde_json::Value| {
            v.as_str()
                .unwrap()
                .trim_start_matches("tenant-")
                .to_string()
        };
        assert_eq!(bare(&seen.body["tenant_id"]), bare(&tenant));
        if what == "start" {
            assert_eq!(seen.body["path"], "calls/x.wav");
        }
    }
    let ended = client.delete(&call_url).send().await.unwrap();
    assert_eq!(ended.status(), 200);
    let seen = asked.lock().unwrap().last().cloned().unwrap();
    assert_eq!(seen.path, format!("/calls/{CALL}/hangup"));
    let missing = client
        .get(loopback(
            http,
            "/api/v1/calls/call-00000000-0000-0000-0000-000000000000",
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), 404);

    // What the API refuses itself is not put to the node.
    let before = asked.lock().unwrap().len();
    let (status, _) = post("transfer", CALL, Some(serde_json::json!({"target": " "}))).await;
    assert_eq!(status, 400);
    let (status, _) = post("say", CALL, Some(serde_json::json!({"text": ""}))).await;
    assert_eq!(status, 400);
    assert_eq!(asked.lock().unwrap().len(), before);

    service.stop().await;
    db.drop_db().await;
}
