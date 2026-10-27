//! A user's own contacts (`docs/RECORD_AND_CONTACTS.md` §3): `/me/contacts`
//! made, read, searched and paged by name, changed and deleted; each user
//! sees only their own; a delete leaves a tombstone no read shows.

use super::*;

/// Signed in as a new end user; their token.
async fn signed_in(app: &App, username: &str) -> String {
    app.create_user_with_role(username, "end_user").await;
    let login = app.login(username, PASSWORD).await;
    assert_eq!(login.status, StatusCode::OK, "{}", login.text);
    login.json["token"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn a_user_keeps_their_own_contacts() {
    let Some(app) = App::build(DeploymentMode::default(), None).await else {
        return;
    };
    let jo = signed_in(&app, "contact-jo").await;
    let sam = signed_in(&app, "contact-sam").await;

    let made = app
        .send_as(
            &jo,
            Method::POST,
            "/api/v1/me/contacts",
            Some(json!({
                "name": "  Zed Zulu ",
                "company": "Acme",
                "numbers": [
                    {"label": "mobile", "number": "0412 345 678"},
                    {"label": "fax", "number": "  "}
                ],
                "email": "",
            })),
        )
        .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.text);
    assert_eq!(made.json["name"], "Zed Zulu", "trimmed");
    assert_eq!(
        made.json["numbers"].as_array().unwrap().len(),
        1,
        "the empty one dropped"
    );
    assert!(made.json["email"].is_null(), "an empty field is none");
    let zed = made.json["id"].as_str().unwrap().to_string();
    for name in ["Amy Adams", "Bob Brown"] {
        let r = app
            .send_as(
                &jo,
                Method::POST,
                "/api/v1/me/contacts",
                Some(json!({"name": name})),
            )
            .await;
        assert_eq!(r.status, StatusCode::CREATED, "{}", r.text);
    }
    let nameless = app
        .send_as(
            &jo,
            Method::POST,
            "/api/v1/me/contacts",
            Some(json!({"name": " "})),
        )
        .await;
    assert_eq!(
        nameless.status,
        StatusCode::BAD_REQUEST,
        "{}",
        nameless.text
    );

    // By name, a page at a time.
    let first = app
        .send_as(&jo, Method::GET, "/api/v1/me/contacts?limit=2", None)
        .await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text);
    let names = |r: &Response| -> Vec<String> {
        r.body["data"]
            .as_array()
            .unwrap_or_else(|| panic!("not a page: {}", r.text))
            .iter()
            .map(|c| c["name"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(names(&first), ["Amy Adams", "Bob Brown"]);
    let cursor = first.body["next_cursor"].as_str().unwrap().to_string();
    let rest = app
        .send_as(
            &jo,
            Method::GET,
            &format!("/api/v1/me/contacts?limit=2&cursor={cursor}"),
            None,
        )
        .await;
    assert_eq!(rest.status, StatusCode::OK, "{}", rest.text);
    assert_eq!(names(&rest), ["Zed Zulu"]);
    assert!(rest.body["next_cursor"].is_null());

    // Found by company and by number.
    for q in ["acme", "345"] {
        let found = app
            .send_as(
                &jo,
                Method::GET,
                &format!("/api/v1/me/contacts?q={q}"),
                None,
            )
            .await;
        assert_eq!(found.status, StatusCode::OK, "q={q}: {}", found.text);
        assert_eq!(names(&found), ["Zed Zulu"], "q={q}");
    }

    // Someone else's contact is not found, read or written.
    let theirs = app
        .send_as(&sam, Method::GET, "/api/v1/me/contacts", None)
        .await;
    assert_eq!(theirs.status, StatusCode::OK, "{}", theirs.text);
    assert!(names(&theirs).is_empty());
    let path = format!("/api/v1/me/contacts/{zed}");
    for (method, body) in [
        (Method::GET, None),
        (Method::PUT, Some(json!({"name": "Mine now"}))),
        (Method::DELETE, None),
    ] {
        let r = app.send_as(&sam, method.clone(), &path, body).await;
        assert_eq!(r.status, StatusCode::NOT_FOUND, "{method}: {}", r.text);
    }

    // Changed: replaced whole, a moment newer.
    let changed = app
        .send_as(
            &jo,
            Method::PUT,
            &path,
            Some(json!({"name": "Zed Zulu", "numbers": [{"label": "work", "number": "1002"}]})),
        )
        .await;
    assert_eq!(changed.status, StatusCode::OK, "{}", changed.text);
    assert!(changed.json["company"].is_null(), "replaced whole");
    assert_eq!(changed.json["numbers"][0]["number"], "1002");
    assert!(changed.json["updated_at"].as_str() > made.json["updated_at"].as_str());

    // Deleted: gone from every read, a tombstone underneath.
    let gone = app.send_as(&jo, Method::DELETE, &path, None).await;
    assert_eq!(gone.status, StatusCode::NO_CONTENT, "{}", gone.text);
    assert_eq!(
        app.send_as(&jo, Method::GET, &path, None).await.status,
        StatusCode::NOT_FOUND
    );
    let left = app
        .send_as(&jo, Method::GET, "/api/v1/me/contacts", None)
        .await;
    assert_eq!(left.status, StatusCode::OK, "{}", left.text);
    assert_eq!(names(&left), ["Amy Adams", "Bob Brown"]);
    let store = app.state.storage.as_ref().unwrap().contact_store().unwrap();
    let tombstone = store.get_contact(&zed).await.unwrap().unwrap();
    assert!(tombstone.deleted_at.is_some());
    app.finish().await;
}

/// A single-tenant deployment need not have a row for its default tenant:
/// its users are still in it, and keep contacts like anyone else.
#[tokio::test]
async fn a_user_in_a_tenant_with_no_row_keeps_contacts() {
    let Some(app) = App::build(DeploymentMode::default(), None).await else {
        return;
    };
    let jo = signed_in(&app, "contact-rowless").await;
    sqlx::query("DELETE FROM tenants WHERE id = $1")
        .bind(app.tenant_a.to_string())
        .execute(app.storage.pool())
        .await
        .unwrap();
    let made = app
        .send_as(
            &jo,
            Method::POST,
            "/api/v1/me/contacts",
            Some(json!({"name": "Kim Park", "numbers": [{"number": "1004"}]})),
        )
        .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.text);
    let list = app
        .send_as(&jo, Method::GET, "/api/v1/me/contacts", None)
        .await;
    assert_eq!(list.body["data"][0]["name"], "Kim Park", "{}", list.text);
    app.finish().await;
}

/// A contact made, changed or deleted reaches the user's apps on
/// `/me/events` as `user.contact_changed`, naming the contact and never
/// what it holds; another user's stream does not carry it.
#[tokio::test]
async fn a_change_to_a_contact_reaches_the_users_apps() {
    use tokio_tungstenite::tungstenite::Message;
    let Some(app) = App::build(DeploymentMode::default(), None).await else {
        return;
    };
    let jo = signed_in(&app, "contact-live").await;
    let sam = signed_in(&app, "contact-other").await;
    let port = app.serve().await;
    let url = format!("ws://127.0.0.1:{port}/api/v1/me/events");
    type Ws = tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >;
    async fn next(ws: &mut Ws, wait: Duration) -> Option<String> {
        let frame = tokio::time::timeout(wait, ws.next()).await.ok()??.ok()?;
        Some(frame.to_text().ok()?.to_string())
    }
    async fn open(url: &str, token: &str) -> Ws {
        let (mut ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();
        ws.send(Message::Text(
            json!({"type": "auth", "token": token}).to_string(),
        ))
        .await
        .unwrap();
        let wait = Duration::from_secs(5);
        assert!(next(&mut ws, wait).await.unwrap().contains("auth_ok"));
        assert!(next(&mut ws, wait).await.unwrap().contains("connected"));
        ws
    }
    let mut mine = open(&url, &jo).await;
    let mut theirs = open(&url, &sam).await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let made = app
        .send_as(
            &jo,
            Method::POST,
            "/api/v1/me/contacts",
            Some(json!({"name": "Secret Name", "numbers": [{"number": "5550101"}]})),
        )
        .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.text);
    let id = made.json["id"].as_str().unwrap().to_string();
    let path = format!("/api/v1/me/contacts/{id}");
    let changed = app
        .send_as(&jo, Method::PUT, &path, Some(json!({"name": "Other Name"})))
        .await;
    assert_eq!(changed.status, StatusCode::OK, "{}", changed.text);
    let gone = app.send_as(&jo, Method::DELETE, &path, None).await;
    assert_eq!(gone.status, StatusCode::NO_CONTENT, "{}", gone.text);

    for change in ["created", "updated", "deleted"] {
        let text = next(&mut mine, Duration::from_secs(5))
            .await
            .unwrap_or_else(|| panic!("no event for {change}"));
        let event: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(event["event_type"], "user", "{text}");
        assert_eq!(event["type"], "contact_changed", "{text}");
        assert_eq!(event["contact_id"], id.as_str(), "{text}");
        assert_eq!(event["change"], change, "{text}");
        assert!(
            !text.contains("Name") && !text.contains("5550101"),
            "{text}"
        );
    }
    assert_eq!(
        next(&mut theirs, Duration::from_millis(500)).await,
        None,
        "another user hears nothing"
    );
    app.finish().await;
}
