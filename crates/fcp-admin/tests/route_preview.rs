//! The admin API's route preview asks a call manager, not itself
//! (`docs/TENANT_TRUNKS.md` §12, phase 6d): `/api/v1/routing/preview` goes
//! to the node `[call_manager] url` names, with its token and the caller's
//! tenant, and hands back what the node said — or why it could not.
//! Skipped without `FCP_TEST_DATABASE_URL`.

use axum::extract::Query;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use fcp_testkit::smoke::{self, free_port, loopback};
use fcp_testkit::{Service, TestDatabase};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

const BIN: &str = env!("CARGO_BIN_EXE_fcp-admin");

/// What the stand-in call manager was asked.
#[derive(Debug, Clone, Default)]
struct Asked {
    params: HashMap<String, String>,
    authorization: Option<String>,
}

/// A call manager's `/routes/preview`, answering from `to`: a number
/// beginning `+44` goes out on one trunk, anything else finds the node
/// with no trunks.
async fn stand_in(asked: Arc<Mutex<Vec<Asked>>>) -> u16 {
    let port = free_port();
    let app = Router::new().route(
        "/routes/preview",
        get(
            move |Query(params): Query<HashMap<String, String>>, headers: HeaderMap| {
                let asked = Arc::clone(&asked);
                async move {
                    asked.lock().unwrap().push(Asked {
                        authorization: headers
                            .get("authorization")
                            .and_then(|v| v.to_str().ok())
                            .map(str::to_string),
                        params: params.clone(),
                    });
                    if params.get("to").is_some_and(|to| to.starts_with("+44")) {
                        (
                            StatusCode::OK,
                            Json(serde_json::json!({
                                "node_id": "cm-1",
                                "outcome": "routes",
                                "routes": [{ "name": "cheap-carrier" }],
                            })),
                        )
                    } else {
                        (
                            StatusCode::SERVICE_UNAVAILABLE,
                            Json(serde_json::json!({ "error": "this node routes by no trunks" })),
                        )
                    }
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
async fn a_route_preview_is_asked_of_the_call_manager() {
    let Some(db) = TestDatabase::create().await else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping");
        return;
    };
    let asked = Arc::new(Mutex::new(Vec::new()));
    let node = stand_in(Arc::clone(&asked)).await;
    let dir = smoke::temp_dir();
    let http = free_port();
    let config = format!(
        r#"[service]
node_id = "preview-admin"

[api]
bind_address = "127.0.0.1:{http}"
auth_enabled = false

[storage]
database_url = "{db}"

prompts_dir = "{dir}/prompts"

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

    // The node's answer, handed back as it gave it.
    let answer = client
        .get(loopback(
            http,
            "/api/v1/routing/preview?to=%2B442079460000&from=1001",
        ))
        .send()
        .await
        .expect("the admin answers");
    let status = answer.status();
    let text = answer.text().await.unwrap();
    assert_eq!(status, 200, "{text}");
    let body: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(body["routes"][0]["name"], "cheap-carrier", "{body}");

    // What the node was asked: this caller's tenant, the number, the
    // caller, and the node's own token.
    let first = asked.lock().unwrap()[0].clone();
    assert_eq!(
        first.params.get("to").map(String::as_str),
        Some("+442079460000")
    );
    assert_eq!(first.params.get("from").map(String::as_str), Some("1001"));
    assert!(
        first.params.get("tenant").is_some_and(|t| !t.is_empty()),
        "{first:?}"
    );
    assert_eq!(first.authorization.as_deref(), Some("Bearer cm-secret"));

    // A node that cannot answer: 503, with its reason.
    let refused = client
        .get(loopback(http, "/api/v1/routing/preview?to=911"))
        .send()
        .await
        .expect("the admin answers");
    assert_eq!(refused.status(), 503);
    let text = refused.text().await.unwrap();
    assert!(text.contains("routes by no trunks"), "{text}");

    service.stop().await;
    db.drop_db().await;
}
