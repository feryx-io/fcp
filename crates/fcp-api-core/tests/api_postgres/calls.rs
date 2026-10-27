//! A call's details, hangup, recording and supervisor through the admin
//! API, route by route (docs/DEEP_TESTING.md D3b): each is put to the call
//! manager holding the call, here a stand-in, with the caller's tenant.
//! The node's side is proved in `fcp-call-manager/tests/calls.rs`, the
//! admin's forwarding in `fcp-admin/tests/call_control.rs`.

use super::*;
use fcp_api_core::{CallSupervisor, SupervisionRequest};
use std::sync::Mutex;

/// The one call the stand-in node holds, in tenant A.
const HELD: &str = "call-11111111-2222-3333-4444-555555555555";

/// What the node was asked: the tenant, the call and the request.
type Asked = Arc<Mutex<Vec<(TenantId, String, String)>>>;

/// A call manager holding [`HELD`] in one tenant: another tenant's
/// request for it, or a call it does not hold, is "Call not found".
struct StandInNode {
    tenant: TenantId,
    asked: Asked,
}

#[async_trait::async_trait]
impl CallSupervisor for StandInNode {
    async fn supervise(
        &self,
        tenant: TenantId,
        call_id: &str,
        request: SupervisionRequest,
    ) -> Result<(u16, Value), String> {
        let what = match &request {
            SupervisionRequest::Show => "show".to_string(),
            SupervisionRequest::Get => "supervisor".to_string(),
            SupervisionRequest::Leave => "leave".to_string(),
            SupervisionRequest::Join { mode, .. } => mode.to_string(),
            SupervisionRequest::Control { action, body } => format!("{action} {body}"),
        };
        self.asked
            .lock()
            .unwrap()
            .push((tenant, call_id.to_string(), what));
        if call_id != HELD || tenant != self.tenant {
            return Ok((
                404,
                json!({ "error": format!("Call not found: {call_id}") }),
            ));
        }
        Ok(match request {
            SupervisionRequest::Show => (
                200,
                json!({
                    "id": HELD, "state": "Established",
                    "from_uri": "sip:alice@pbx.example.com", "to_uri": "sip:bob@pbx.example.com",
                    "from_display_name": "Alice", "to_display_name": null,
                    "created_at": "2026-10-16T09:00:00.000Z",
                    "established_at": "2026-10-16T09:00:02.000Z", "ended_at": null,
                    "duration_seconds": 40, "media_anchored": true, "failure_reason": null,
                }),
            ),
            SupervisionRequest::Get => (
                200,
                json!({ "supervisor": "1009", "mode": "whisper", "heard_by": "a" }),
            ),
            SupervisionRequest::Leave => (200, json!({ "call_id": HELD, "status": "left" })),
            SupervisionRequest::Join { mode, .. } => {
                (200, json!({ "call_id": HELD, "status": mode }))
            }
            SupervisionRequest::Control { action, body } => match action {
                "recording/start" if body["path"] == "../out.wav" => (
                    400,
                    json!({ "error": "the path leaves the tenant's directory" }),
                ),
                "recording/start" => (
                    200,
                    json!({ "call_id": HELD, "status": "recording", "recording_id": "rec-1",
                            "path": "/recordings/t/rec-1.wav", "consent": "not_required" }),
                ),
                "recording/stop" => (200, json!({ "call_id": HELD, "status": "stopped" })),
                "hangup" => (200, json!({ "call_id": HELD, "status": "ended" })),
                other => (400, json!({ "error": format!("unexpected {other}") })),
            },
        })
    }
}

impl App {
    /// The app with a call manager to put calls to.
    fn with_node(mut self, asked: &Asked) -> Self {
        self.state = self
            .state
            .clone()
            .with_call_supervisor(Arc::new(StandInNode {
                tenant: self.tenant_a,
                asked: Arc::clone(asked),
            }));
        self.router = create_router(self.state.clone(), &test_api_config());
        self
    }
}

/// Each route reaches the node with the caller's tenant and the node's
/// answer comes back; another tenant's administrator asking for the call
/// is told it is not found, by the node, for their own tenant.
#[tokio::test]
async fn a_calls_routes_are_put_to_the_node_holding_it() {
    let asked: Asked = Default::default();
    let Some(app) = App::build(DeploymentMode::default(), None).await else {
        return;
    };
    let app = app.with_node(&asked);
    let call = format!("/api/v1/calls/{HELD}");

    let shown = app.admin(Method::GET, &call, None).await;
    assert_eq!(shown.status, StatusCode::OK, "{}", shown.text);
    assert_eq!(shown.json["id"], HELD);
    assert_eq!(shown.json["from_display_name"], "Alice");

    let started = app
        .admin(
            Method::POST,
            &format!("{call}/recording/start"),
            Some(json!({ "path": "calls/one.wav" })),
        )
        .await;
    assert_eq!(started.status, StatusCode::OK, "{}", started.text);
    assert_eq!(started.json["status"], "recording");
    assert_eq!(started.json["recording_id"], "rec-1");
    let plain = app
        .send(
            Method::POST,
            &format!("{call}/recording/start"),
            &[("authorization", &format!("Bearer {ADMIN_TOKEN}"))],
            None,
        )
        .await;
    assert_eq!(plain.status, StatusCode::OK, "no body: {}", plain.text);
    let escape = app
        .admin(
            Method::POST,
            &format!("{call}/recording/start"),
            Some(json!({ "path": "../out.wav" })),
        )
        .await;
    assert_eq!(escape.status, StatusCode::BAD_REQUEST, "{}", escape.text);
    assert_eq!(escape.json["code"], "bad_request", "{}", escape.text);
    let stopped = app
        .admin(Method::POST, &format!("{call}/recording/stop"), None)
        .await;
    assert_eq!(stopped.status, StatusCode::OK, "{}", stopped.text);
    assert_eq!(stopped.json["status"], "stopped");

    let whisper = app
        .admin(
            Method::POST,
            &format!("{call}/whisper"),
            Some(json!({ "supervisor": "1009", "heard_by": "a" })),
        )
        .await;
    assert_eq!(whisper.status, StatusCode::OK, "{}", whisper.text);
    assert_eq!(whisper.json["status"], "whisper");
    let supervisor = app
        .admin(Method::GET, &format!("{call}/supervisor"), None)
        .await;
    assert_eq!(supervisor.status, StatusCode::OK, "{}", supervisor.text);
    assert_eq!(supervisor.json["supervisor"], "1009");
    let left = app
        .admin(Method::DELETE, &format!("{call}/supervisor"), None)
        .await;
    assert_eq!(left.status, StatusCode::OK, "{}", left.text);

    let ended = app.admin(Method::DELETE, &call, None).await;
    assert_eq!(ended.status, StatusCode::OK, "{}", ended.text);
    assert_eq!(ended.json["status"], "ended");

    // What reached the node, in order, every one for tenant A.
    let seen: Vec<String> = asked
        .lock()
        .unwrap()
        .iter()
        .map(|(tenant, id, what)| {
            assert_eq!(*tenant, app.tenant_a);
            assert_eq!(id, HELD);
            what.clone()
        })
        .collect();
    assert_eq!(
        seen,
        vec![
            "show".to_string(),
            r#"recording/start {"path":"calls/one.wav"}"#.to_string(),
            "recording/start {}".to_string(),
            r#"recording/start {"path":"../out.wav"}"#.to_string(),
            "recording/stop {}".to_string(),
            "whisper".to_string(),
            "supervisor".to_string(),
            "leave".to_string(),
            "hangup {}".to_string(),
        ]
    );

    // Tenant B's administrator is asked about in tenant B, where there is
    // no such call.
    asked.lock().unwrap().clear();
    for (method, path) in [
        (Method::GET, call.clone()),
        (Method::DELETE, call.clone()),
        (Method::POST, format!("{call}/recording/start")),
        (Method::POST, format!("{call}/recording/stop")),
        (Method::GET, format!("{call}/supervisor")),
    ] {
        let res = app
            .admin_in(app.tenant_b, method.clone(), &path, None)
            .await;
        assert_eq!(
            res.status,
            StatusCode::NOT_FOUND,
            "{method} {path}: {}",
            res.text
        );
        assert_eq!(res.json["code"], "not_found", "{}", res.text);
    }
    assert!(asked
        .lock()
        .unwrap()
        .iter()
        .all(|(tenant, _, _)| *tenant == app.tenant_b));
    app.finish().await;
}

/// What the API refuses itself never reaches the node: a body that does
/// not parse, a role without the permission.
#[tokio::test]
async fn a_refused_request_is_not_put_to_the_node() {
    let asked: Asked = Default::default();
    let Some(app) = App::build(DeploymentMode::default(), None).await else {
        return;
    };
    let app = app.with_node(&asked);
    let call = format!("/api/v1/calls/{HELD}");
    let bad = app
        .send(
            Method::POST,
            &format!("{call}/recording/start"),
            &[
                ("authorization", &format!("Bearer {ADMIN_TOKEN}")),
                ("content-type", "application/json"),
            ],
            Some(json!("not an object")),
        )
        .await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST, "{}", bad.text);

    app.create_user_with_role("ro", "read_only").await;
    let ro = session(&app, "ro").await;
    let shown = app.send_as(&ro, Method::GET, &call, None).await;
    assert_eq!(shown.status, StatusCode::OK, "{}", shown.text);
    for (method, path) in [
        (Method::DELETE, call.clone()),
        (Method::POST, format!("{call}/recording/start")),
        (Method::POST, format!("{call}/recording/stop")),
        (Method::GET, format!("{call}/supervisor")),
    ] {
        let res = app.send_as(&ro, method.clone(), &path, None).await;
        assert_eq!(
            res.status,
            StatusCode::FORBIDDEN,
            "{method} {path}: {}",
            res.text
        );
    }
    let seen: Vec<String> = asked.lock().unwrap().iter().map(|a| a.2.clone()).collect();
    assert_eq!(seen, vec!["show".to_string()]);
    app.finish().await;
}

/// Without a call manager to ask, the routes say so.
#[tokio::test]
async fn without_a_call_manager_a_call_cannot_be_reached() {
    let app = require_app!();
    let call = format!("/api/v1/calls/{HELD}");
    for (method, path) in [
        (Method::GET, call.clone()),
        (Method::DELETE, call.clone()),
        (Method::POST, format!("{call}/recording/start")),
        (Method::POST, format!("{call}/recording/stop")),
    ] {
        let res = app.admin(method.clone(), &path, None).await;
        assert_eq!(
            res.status,
            StatusCode::SERVICE_UNAVAILABLE,
            "{method} {path}: {}",
            res.text
        );
    }
    app.finish().await;
}

/// A user records a call they are on from their app
/// (`docs/RECORD_AND_CONTACTS.md` §2): refused until an administrator turns
/// `record_on_demand` on, then put to the node naming them in `for_user`,
/// answered without the file's path. Whether they are on the call is the
/// node's to say.
#[tokio::test]
async fn a_user_records_their_own_call_once_an_administrator_lets_them() {
    let asked: Asked = Default::default();
    let Some(app) = App::build(DeploymentMode::default(), None).await else {
        return;
    };
    let app = app.with_node(&asked);
    let user_id = app.create_user_with_role("recorder", "end_user").await;
    let login = app.login("recorder", PASSWORD).await;
    assert_eq!(login.status, StatusCode::OK, "{}", login.text);
    let token = login.json["token"].as_str().unwrap().to_string();
    let start = format!("/api/v1/me/calls/{HELD}/recording/start");
    let stop = format!("/api/v1/me/calls/{HELD}/recording/stop");

    let refused = app.send_as(&token, Method::POST, &start, None).await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.text);
    assert!(asked.lock().unwrap().is_empty(), "the node was not asked");

    let shown = app.admin(Method::GET, "/api/v1/users/recorder", None).await;
    assert_eq!(shown.json["media_preferences"]["record_on_demand"], false);
    let mut prefs = shown.json["media_preferences"].clone();
    prefs["record_on_demand"] = json!(true);
    let updated = app
        .admin(
            Method::PUT,
            "/api/v1/users/recorder",
            Some(json!({ "media_preferences": prefs })),
        )
        .await;
    assert_eq!(updated.status, StatusCode::OK, "{}", updated.text);
    assert_eq!(updated.json["media_preferences"]["record_on_demand"], true);

    let started = app.send_as(&token, Method::POST, &start, None).await;
    assert_eq!(started.status, StatusCode::OK, "{}", started.text);
    assert_eq!(started.json["status"], "recording");
    assert_eq!(started.json["recording_id"], "rec-1");
    assert!(started.json.get("path").is_none(), "{}", started.text);
    let stopped = app.send_as(&token, Method::POST, &stop, None).await;
    assert_eq!(stopped.status, StatusCode::OK, "{}", stopped.text);

    let elsewhere = app
        .send_as(
            &token,
            Method::POST,
            "/api/v1/me/calls/call-00000000-0000-4000-8000-000000000000/recording/start",
            None,
        )
        .await;
    assert_eq!(
        elsewhere.status,
        StatusCode::NOT_FOUND,
        "{}",
        elsewhere.text
    );

    let bare = user_id.trim_start_matches("user-");
    let for_user = format!(r#"{{"for_user":"{bare}"}}"#);
    let asked = asked.lock().unwrap().clone();
    assert_eq!(asked[0].0, app.tenant_a);
    assert_eq!(asked[0].2, format!("recording/start {for_user}"));
    assert_eq!(asked[1].2, format!("recording/stop {for_user}"));
    app.finish().await;
}
