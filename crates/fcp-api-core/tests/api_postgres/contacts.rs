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

/// A contact's ringtone is kept, kept through a change that does not name
/// it (an older app, the portal), and cleared by `null`.
#[tokio::test]
async fn a_contacts_ringtone_is_kept_until_cleared() {
    let Some(app) = App::build(DeploymentMode::default(), None).await else {
        return;
    };
    let jo = signed_in(&app, "contact-tone").await;
    let made = app
        .send_as(
            &jo,
            Method::POST,
            "/api/v1/me/contacts",
            Some(json!({"name": "Kim Park", "ringtone": "classic"})),
        )
        .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.text);
    assert_eq!(made.json["ringtone"], "classic");
    let path = format!("/api/v1/me/contacts/{}", made.json["id"].as_str().unwrap());
    let changed = app
        .send_as(
            &jo,
            Method::PUT,
            &path,
            Some(json!({"name": "Kim Park Jr"})),
        )
        .await;
    assert_eq!(changed.status, StatusCode::OK, "{}", changed.text);
    assert_eq!(changed.json["ringtone"], "classic", "not named: kept");
    let cleared = app
        .send_as(
            &jo,
            Method::PUT,
            &path,
            Some(json!({"name": "Kim Park Jr", "ringtone": null})),
        )
        .await;
    assert!(cleared.json["ringtone"].is_null(), "{}", cleared.text);
    app.finish().await;
}

/// Contacts go out as a vCard or a CSV file and come in from one: a file
/// from a phone made into contacts, what is already kept not made again,
/// an entry with nothing to call it by skipped by its place, and the apps
/// told once.
#[tokio::test]
async fn contacts_are_imported_and_exported_as_files() {
    let Some(app) = App::build(DeploymentMode::default(), None).await else {
        return;
    };
    let jo = signed_in(&app, "contact-files").await;
    let kept = app
        .send_as(
            &jo,
            Method::POST,
            "/api/v1/me/contacts",
            Some(json!({"name": "Amy Adams", "numbers": [{"label": "work", "number": "1001"}]})),
        )
        .await;
    assert_eq!(kept.status, StatusCode::CREATED, "{}", kept.text);

    let vcard = "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Amy Adams\r\nTEL;TYPE=WORK:1001\r\nEND:VCARD\r\n\
                 BEGIN:VCARD\r\nVERSION:3.0\r\nN:Park;Kim;;;\r\nORG:Boats\r\n\
                 TEL;TYPE=CELL:0400 111 222\r\nitem1.TEL:555\r\nitem1.X-ABLabel:Boat\r\n\
                 EMAIL:kim@example.com\r\nEND:VCARD\r\n\
                 BEGIN:VCARD\r\nVERSION:3.0\r\nNOTE:nobody\r\nEND:VCARD\r\n";
    let imported = app
        .send_raw(
            &jo,
            Method::POST,
            "/api/v1/me/contacts/import",
            "text/vcard",
            vcard.as_bytes().to_vec(),
        )
        .await;
    assert_eq!(imported.status, StatusCode::OK, "{}", imported.text);
    assert_eq!(imported.json["imported"], 1, "{}", imported.text);
    assert_eq!(imported.json["duplicates"], 1, "Amy is kept already");
    assert_eq!(imported.json["skipped"][0]["entry"], 3, "{}", imported.text);

    let list = app
        .send_as(&jo, Method::GET, "/api/v1/me/contacts", None)
        .await;
    let kim = list
        .json
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "Kim Park")
        .cloned()
        .unwrap();
    assert_eq!(kim["company"], "Boats");
    assert_eq!(
        kim["numbers"][0],
        json!({"label": "mobile", "number": "0400 111 222"})
    );
    assert_eq!(kim["numbers"][1], json!({"label": "Boat", "number": "555"}));

    // Out as CSV, then in again: nothing new.
    let csv = app
        .send_as(
            &jo,
            Method::GET,
            "/api/v1/me/contacts/export?format=csv",
            None,
        )
        .await;
    assert_eq!(csv.status, StatusCode::OK, "{}", csv.text);
    assert!(csv.headers[header::CONTENT_TYPE]
        .to_str()
        .unwrap()
        .starts_with("text/csv"));
    assert!(
        csv.text.starts_with("Name,Organization 1 - Name"),
        "{}",
        csv.text
    );
    assert!(
        csv.text
            .contains("Kim Park,Boats,kim@example.com,,Mobile,0400 111 222,Boat,555"),
        "{}",
        csv.text
    );
    let again = app
        .send_raw(
            &jo,
            Method::POST,
            "/api/v1/me/contacts/import?format=csv",
            "text/csv",
            csv.bytes.clone(),
        )
        .await;
    assert_eq!(again.status, StatusCode::OK, "{}", again.text);
    assert_eq!(again.json["imported"], 0);
    assert_eq!(again.json["duplicates"], 2);

    let vcf = app
        .send_as(&jo, Method::GET, "/api/v1/me/contacts/export", None)
        .await;
    assert!(vcf.headers[header::CONTENT_DISPOSITION]
        .to_str()
        .unwrap()
        .contains("contacts.vcf"));
    assert_eq!(vcf.text.matches("BEGIN:VCARD").count(), 2, "{}", vcf.text);
    let bad = app
        .send_as(
            &jo,
            Method::GET,
            "/api/v1/me/contacts/export?format=pdf",
            None,
        )
        .await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST);
    app.finish().await;
}

/// A user an administrator takes out of the directory is not in it for
/// the tenant's users; a user changes their own listing only when the
/// tenant lets them.
#[tokio::test]
async fn a_user_taken_out_of_the_directory_is_not_listed() {
    let Some(app) = App::build(DeploymentMode::default(), None).await else {
        return;
    };
    let viewer = signed_in(&app, "dir-viewer").await;
    let exec = signed_in(&app, "dir-exec").await;
    let names = |r: &Response| -> Vec<String> {
        r.json
            .as_array()
            .unwrap_or_else(|| panic!("not a list: {}", r.text))
            .iter()
            .filter_map(|e| e["username"].as_str().map(String::from))
            .collect()
    };
    let listed = app
        .send_as(&viewer, Method::GET, "/api/v1/me/directory", None)
        .await;
    assert!(
        names(&listed).contains(&"dir-exec".to_string()),
        "{}",
        listed.text
    );

    // The administrator unlists them.
    let hidden = app
        .admin(
            Method::PUT,
            "/api/v1/users/dir-exec",
            Some(json!({"directory_listed": false})),
        )
        .await;
    assert_eq!(hidden.status, StatusCode::OK, "{}", hidden.text);
    assert_eq!(hidden.json["directory_listed"], false);
    let listed = app
        .send_as(&viewer, Method::GET, "/api/v1/me/directory", None)
        .await;
    assert!(
        !names(&listed).contains(&"dir-exec".to_string()),
        "{}",
        listed.text
    );

    // Not theirs to change unless the tenant says so.
    let profile = app
        .send_as(&exec, Method::GET, "/api/v1/me/profile", None)
        .await;
    assert_eq!(profile.json["directory_listed"], false);
    assert_eq!(profile.json["may_change_directory_listing"], false);
    let refused = app
        .send_as(
            &exec,
            Method::PUT,
            "/api/v1/me/profile",
            Some(json!({"directory_listed": true})),
        )
        .await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.text);
    let allowed = app
        .admin(
            Method::PUT,
            &format!("/api/v1/tenants/{}", app.tenant_a),
            Some(json!({"directory": {"visible_to_users": true, "users_may_unlist": true}})),
        )
        .await;
    assert_eq!(allowed.status, StatusCode::OK, "{}", allowed.text);
    let back = app
        .send_as(
            &exec,
            Method::PUT,
            "/api/v1/me/profile",
            Some(json!({"directory_listed": true})),
        )
        .await;
    assert_eq!(back.status, StatusCode::OK, "{}", back.text);
    assert_eq!(back.json["directory_listed"], true);
    assert_eq!(back.json["may_change_directory_listing"], true);
    let listed = app
        .send_as(&viewer, Method::GET, "/api/v1/me/directory", None)
        .await;
    assert!(
        names(&listed).contains(&"dir-exec".to_string()),
        "{}",
        listed.text
    );
    app.finish().await;
}
