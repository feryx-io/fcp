//! Meetings by id, the caller's own, and live video, route by route
//! (docs/DEEP_TESTING.md D3b).

use super::*;

impl App {
    /// The app able to mint browser join tokens (`[conference] join_secret`).
    fn with_join_secret(mut self) -> Self {
        self.state = self
            .state
            .clone()
            .with_conference_join_secret(Some("join-s3cret".to_string()));
        self.router = create_router(self.state.clone(), &test_api_config());
        self
    }
}

async fn new_meeting(app: &App, tenant: TenantId, room: &str, body: Value) -> String {
    let mut req = json!({ "room_number": room, "name": format!("Room {room}") });
    req.as_object_mut()
        .unwrap()
        .extend(body.as_object().unwrap().clone());
    let res = app
        .admin_in(
            tenant,
            Method::POST,
            "/api/v1/conferences/meetings",
            Some(req),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.text);
    res.json["id"].as_str().unwrap().to_string()
}

/// A meeting is read, changed (a PIN cleared with `null`), turned off and
/// on, given a join token and deleted by id, in its own tenant only; its
/// owner must be one of the tenant's users.
#[tokio::test]
async fn a_meeting_is_managed_by_id_in_its_tenant() {
    let app = require_app!();
    let unconfigured = {
        let id = new_meeting(&app, app.tenant_a, "8199", json!({})).await;
        app.admin(
            Method::POST,
            &format!("/api/v1/conferences/meetings/{id}/join-tokens"),
            Some(json!({})),
        )
        .await
    };
    assert_eq!(
        unconfigured.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        unconfigured.text
    );
    let app = app.with_join_secret();
    let olga_id = app.create_user_with_role("olga", "end_user").await;
    let id = new_meeting(
        &app,
        app.tenant_a,
        "8100",
        json!({ "guest_pin": "1234", "host_pin": "9876", "owner_id": "olga" }),
    )
    .await;
    let theirs = new_meeting(&app, app.tenant_b, "8100", json!({})).await;
    let path = format!("/api/v1/conferences/meetings/{id}");

    let got = app.admin(Method::GET, &path, None).await;
    assert_eq!(got.status, StatusCode::OK, "{}", got.text);
    assert_eq!(got.json["room_number"], "8100");
    assert_eq!(got.json["guest_pin"], "1234");
    assert_eq!(got.json["owner_id"], "olga");
    let bad_id = app
        .admin(Method::GET, "/api/v1/conferences/meetings/not-an-id", None)
        .await;
    assert_eq!(bad_id.status, StatusCode::BAD_REQUEST, "{}", bad_id.text);

    // Another tenant's meeting, by every route, is not found.
    let other = format!("/api/v1/conferences/meetings/{theirs}");
    for (method, path, body) in [
        (Method::GET, other.clone(), None),
        (
            Method::PATCH,
            other.clone(),
            Some(json!({ "name": "mine" })),
        ),
        (Method::POST, format!("{other}/enable"), None),
        (Method::POST, format!("{other}/disable"), None),
        (
            Method::POST,
            format!("{other}/join-tokens"),
            Some(json!({})),
        ),
        (Method::DELETE, other.clone(), None),
    ] {
        let res = app.admin(method.clone(), &path, body).await;
        assert_eq!(
            res.status,
            StatusCode::NOT_FOUND,
            "{method} {path}: {}",
            res.text
        );
    }

    let changed = app
        .admin(
            Method::PATCH,
            &path,
            Some(
                json!({ "name": "Board", "guest_pin": null, "max_participants": 12,
                         "video_enabled": true, "video_layout": "active_speaker",
                         "video_mode": "auto" }),
            ),
        )
        .await;
    assert_eq!(changed.status, StatusCode::OK, "{}", changed.text);
    assert_eq!(changed.json["name"], "Board");
    assert_eq!(changed.json["guest_pin"], Value::Null, "{}", changed.text);
    assert_eq!(changed.json["host_pin"], "9876", "untouched");
    assert_eq!(changed.json["max_participants"], 12);
    assert_eq!(changed.json["video_mode"], "auto", "{}", changed.text);
    assert_eq!(got.json["video_mode"], "mix", "the default");
    let bad_mode = app
        .admin(Method::PATCH, &path, Some(json!({ "video_mode": "sfu" })))
        .await;
    assert_eq!(
        bad_mode.status,
        StatusCode::BAD_REQUEST,
        "{}",
        bad_mode.text
    );
    let bad_video = app
        .admin(
            Method::PATCH,
            &path,
            Some(json!({ "video_layout": "kaleidoscope" })),
        )
        .await;
    assert_eq!(
        bad_video.status,
        StatusCode::BAD_REQUEST,
        "{}",
        bad_video.text
    );
    for owner in ["nobody", "olga@elsewhere"] {
        let res = app
            .admin(Method::PATCH, &path, Some(json!({ "owner_id": owner })))
            .await;
        assert_eq!(res.status, StatusCode::BAD_REQUEST, "{owner}: {}", res.text);
    }
    let by_id = app
        .admin(Method::PATCH, &path, Some(json!({ "owner_id": olga_id })))
        .await;
    assert_eq!(
        by_id.json["owner_id"], "olga",
        "kept by username: {}",
        by_id.text
    );
    let unowned = app
        .admin(Method::PATCH, &path, Some(json!({ "owner_id": null })))
        .await;
    assert_eq!(unowned.json["owner_id"], Value::Null, "{}", unowned.text);

    let off = app
        .admin(Method::POST, &format!("{path}/disable"), None)
        .await;
    assert_eq!(off.status, StatusCode::OK, "{}", off.text);
    assert_eq!(off.json["enabled"], false);
    let on = app
        .admin(Method::POST, &format!("{path}/enable"), None)
        .await;
    assert_eq!(on.json["enabled"], true, "{}", on.text);

    let token = app
        .admin(
            Method::POST,
            &format!("{path}/join-tokens"),
            Some(json!({ "display_name": "Guest" })),
        )
        .await;
    assert_eq!(token.status, StatusCode::CREATED, "{}", token.text);
    assert!(token.json["token"].is_string(), "{}", token.text);
    assert_eq!(token.json["room_number"], "8100");
    assert_eq!(token.json["display_name"], "Guest");
    assert_eq!(token.json["role"], "guest");

    let deleted = app.admin(Method::DELETE, &path, None).await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT, "{}", deleted.text);
    let gone = app.admin(Method::GET, &path, None).await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND, "{}", gone.text);
    let still = app.admin_in(app.tenant_b, Method::GET, &other, None).await;
    assert_eq!(still.status, StatusCode::OK, "{}", still.text);
    app.finish().await;
}

/// A meeting's owner, made in the tenant, must be one of its users.
#[tokio::test]
async fn a_meeting_is_owned_by_one_of_the_tenants_users() {
    let app = require_app!();
    app.create_user_with_role("olga", "end_user").await;
    for owner in ["nobody", "olga@elsewhere", ""] {
        let res = app
            .admin(
                Method::POST,
                "/api/v1/conferences/meetings",
                Some(json!({ "room_number": "8200", "name": "x", "owner_id": owner })),
            )
            .await;
        assert_eq!(
            res.status,
            StatusCode::BAD_REQUEST,
            "{owner:?}: {}",
            res.text
        );
    }
    // Tenant B has no olga.
    let res = app
        .admin_in(
            app.tenant_b,
            Method::POST,
            "/api/v1/conferences/meetings",
            Some(json!({ "room_number": "8200", "name": "x", "owner_id": "olga" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST, "{}", res.text);
    app.finish().await;
}

/// The caller's own meetings: listed, read, changed within what an owner
/// may change, and given join tokens; someone else's, in their tenant or
/// another, is not found.
#[tokio::test]
async fn an_owner_manages_their_own_meetings() {
    let app = require_app!().with_join_secret();
    app.create_user_with_role("olga", "end_user").await;
    app.create_user_with_role("pete", "end_user").await;
    let mine = new_meeting(
        &app,
        app.tenant_a,
        "8301",
        json!({ "owner_id": "olga", "guest_pin": "1111", "host_pin": "2222" }),
    )
    .await;
    let also_mine = new_meeting(&app, app.tenant_a, "8302", json!({ "owner_id": "olga" })).await;
    let petes = new_meeting(&app, app.tenant_a, "8303", json!({ "owner_id": "pete" })).await;
    let unowned = new_meeting(&app, app.tenant_a, "8304", json!({})).await;
    let olga = session(&app, "olga").await;

    let listed = app
        .send_as(&olga, Method::GET, "/api/v1/me/conferences", None)
        .await;
    assert_eq!(listed.status, StatusCode::OK, "{}", listed.text);
    let ids: Vec<&str> = listed
        .json
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        vec![mine.as_str(), also_mine.as_str()],
        "{}",
        listed.text
    );

    let path = format!("/api/v1/me/conferences/{mine}");
    let got = app.send_as(&olga, Method::GET, &path, None).await;
    assert_eq!(got.status, StatusCode::OK, "{}", got.text);
    assert_eq!(got.json["room_number"], "8301");
    for id in [&petes, &unowned] {
        let path = format!("/api/v1/me/conferences/{id}");
        for (method, path, body) in [
            (Method::GET, path.clone(), None),
            (
                Method::PATCH,
                path.clone(),
                Some(json!({ "name": "mine now" })),
            ),
            (Method::POST, format!("{path}/join-tokens"), None),
        ] {
            let res = app.send_as(&olga, method.clone(), &path, body).await;
            assert_eq!(
                res.status,
                StatusCode::NOT_FOUND,
                "{method} {path}: {}",
                res.text
            );
        }
    }

    let changed = app
        .send_as(
            &olga,
            Method::PATCH,
            &path,
            Some(json!({ "name": "Olga's room", "guest_pin": null, "wait_for_moderator": true })),
        )
        .await;
    assert_eq!(changed.status, StatusCode::OK, "{}", changed.text);
    assert_eq!(changed.json["name"], "Olga's room");
    assert_eq!(changed.json["guest_pin"], Value::Null, "{}", changed.text);
    assert_eq!(changed.json["host_pin"], "2222");
    assert_eq!(changed.json["wait_for_moderator"], true);
    // What only an administrator changes is not the owner's to.
    let not_theirs = app
        .send_as(
            &olga,
            Method::PATCH,
            &path,
            Some(json!({ "owner_id": "pete", "enabled": false })),
        )
        .await;
    assert_eq!(
        not_theirs.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        not_theirs.text
    );
    let after = app.send_as(&olga, Method::GET, &path, None).await;
    assert_eq!(after.json["owner_id"], "olga", "{}", after.text);
    assert_eq!(after.json["enabled"], true);

    let empty = app
        .send_as(&olga, Method::POST, &format!("{path}/join-tokens"), None)
        .await;
    assert_eq!(empty.status, StatusCode::CREATED, "{}", empty.text);
    let named = app
        .send_as(
            &olga,
            Method::POST,
            &format!("{path}/join-tokens"),
            Some(json!({ "display_name": "Visitor" })),
        )
        .await;
    assert_eq!(named.status, StatusCode::CREATED, "{}", named.text);
    let broken = app
        .send_raw(
            &olga,
            Method::POST,
            &format!("{path}/join-tokens"),
            "application/json",
            b"{not json".to_vec(),
        )
        .await;
    assert_eq!(broken.status, StatusCode::BAD_REQUEST, "{}", broken.text);
    app.finish().await;
}

/// Screen-share controls reach the conference server for the meeting's
/// tenant; a content layout there is none of is refused here.
#[tokio::test]
async fn a_hosts_screen_share_controls_reach_the_conference_server() {
    let (url, seen) = mock_conference_server().await;
    let Some(app) = App::with_conference_server(&url).await else {
        return;
    };
    new_meeting(&app, app.tenant_a, "8400", json!({})).await;
    new_meeting(&app, app.tenant_b, "8401", json!({})).await;

    let stop = app
        .admin(
            Method::POST,
            "/api/v1/conferences/live/8400/video/content/stop",
            None,
        )
        .await;
    assert_eq!(stop.status, StatusCode::OK, "{}", stop.text);
    assert_eq!(stop.json["path"], "/conferences/8400/video/content/stop");
    let layout = app
        .admin(
            Method::PUT,
            "/api/v1/conferences/live/8400/video/content-layout",
            Some(json!({ "layout": "content_only" })),
        )
        .await;
    assert_eq!(layout.status, StatusCode::OK, "{}", layout.text);
    assert_eq!(layout.json["body"], json!({ "layout": "content_only" }));
    {
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert!(seen
            .iter()
            .all(|up| up.tenant.as_deref() == Some(app.tenant_a.to_string().as_str())));
        assert_eq!(seen[1].method, "PUT");
    }

    let bad = app
        .admin(
            Method::PUT,
            "/api/v1/conferences/live/8400/video/content-layout",
            Some(json!({ "layout": "mosaic" })),
        )
        .await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST, "{}", bad.text);
    for (method, path, body) in [
        (
            Method::POST,
            "/api/v1/conferences/live/8401/video/content/stop",
            None,
        ),
        (
            Method::PUT,
            "/api/v1/conferences/live/8401/video/content-layout",
            Some(json!({ "layout": "presentation" })),
        ),
    ] {
        let res = app.admin(method.clone(), path, body).await;
        assert_eq!(
            res.status,
            StatusCode::NOT_FOUND,
            "{method} {path}: {}",
            res.text
        );
    }
    assert_eq!(seen.lock().unwrap().len(), 2, "nothing more reached it");
    app.finish().await;
}

/// A host controls their room from their app (`docs/SOFTPHONE_VIDEO.md`
/// §2.6): the meeting's owner, or whoever shows a host token for that
/// room; a guest's token, a host token for another room, or none is 403.
#[tokio::test]
async fn a_host_controls_their_room_from_their_own_session() {
    let (url, seen) = mock_conference_server().await;
    let Some(app) = App::with_conference_server(&url).await else {
        return;
    };
    let app = app.with_join_secret();
    app.create_user_with_role("olga", "end_user").await;
    app.create_user_with_role("pete", "end_user").await;
    let room = new_meeting(&app, app.tenant_a, "8402", json!({ "owner_id": "olga" })).await;
    let other = new_meeting(&app, app.tenant_a, "8403", json!({ "owner_id": "olga" })).await;
    let olga = session(&app, "olga").await;
    let pete = session(&app, "pete").await;

    let base = "/api/v1/me/conferences/live/8402";
    let controls: Vec<(Method, String, Option<Value>, &str, &str)> = vec![
        (
            Method::GET,
            base.to_string(),
            None,
            "GET",
            "/conferences/8402",
        ),
        (
            Method::PUT,
            format!("{base}/video/layout"),
            Some(json!({ "layout": "grid" })),
            "PUT",
            "/conferences/8402/video/layout",
        ),
        (
            Method::POST,
            format!("{base}/video/pin/p1"),
            None,
            "POST",
            "/conferences/8402/video/pin/p1",
        ),
        (
            Method::DELETE,
            format!("{base}/video/pin"),
            None,
            "DELETE",
            "/conferences/8402/video/pin",
        ),
        (
            Method::POST,
            format!("{base}/video/spotlight/p1"),
            None,
            "POST",
            "/conferences/8402/video/spotlight/p1",
        ),
        (
            Method::DELETE,
            format!("{base}/video/spotlight"),
            None,
            "DELETE",
            "/conferences/8402/video/spotlight",
        ),
        (
            Method::POST,
            format!("{base}/participants/p1/mute"),
            None,
            "POST",
            "/conferences/8402/mute/p1",
        ),
        (
            Method::POST,
            format!("{base}/participants/p1/unmute"),
            None,
            "POST",
            "/conferences/8402/unmute/p1",
        ),
        (
            Method::POST,
            format!("{base}/participants/p1/video/off"),
            None,
            "POST",
            "/conferences/8402/participants/p1/video/off",
        ),
        (
            Method::DELETE,
            format!("{base}/participants/p1"),
            None,
            "POST",
            "/conferences/8402/kick/p1",
        ),
        (
            Method::POST,
            format!("{base}/video/content/stop"),
            None,
            "POST",
            "/conferences/8402/video/content/stop",
        ),
        // The audience's voice (`docs/WEBINARS.md` §2.5).
        (
            Method::DELETE,
            format!("{base}/participants/p1/hand"),
            None,
            "DELETE",
            "/conferences/8402/participants/p1/hand",
        ),
        (
            Method::DELETE,
            format!("{base}/hands"),
            None,
            "DELETE",
            "/conferences/8402/hands",
        ),
        (
            Method::POST,
            format!("{base}/participants/p1/stage"),
            None,
            "POST",
            "/conferences/8402/participants/p1/stage",
        ),
        (
            Method::DELETE,
            format!("{base}/participants/p1/stage"),
            None,
            "DELETE",
            "/conferences/8402/participants/p1/stage",
        ),
    ];
    for (method, path, body, up_method, up_path) in &controls {
        let res = app.send_as(&olga, method.clone(), path, body.clone()).await;
        assert_eq!(res.status, StatusCode::OK, "{method} {path}: {}", res.text);
        assert_eq!(res.json["path"], *up_path, "{method} {path}");
        let last = seen.lock().unwrap().last().cloned().unwrap();
        assert_eq!(last.method, *up_method, "{method} {path}");
        assert_eq!(
            last.tenant.as_deref(),
            Some(app.tenant_a.to_string().as_str())
        );
    }
    let reached = seen.lock().unwrap().len();
    assert_eq!(reached, controls.len());

    // Olga's tokens for her rooms, shown by Pete.
    let mint = |id: String, role: &'static str| {
        let app = &app;
        let olga = olga.clone();
        async move {
            let res = app
                .send_as(
                    &olga,
                    Method::POST,
                    &format!("/api/v1/me/conferences/{id}/join-tokens"),
                    Some(json!({ "role": role })),
                )
                .await;
            assert_eq!(res.status, StatusCode::CREATED, "{}", res.text);
            res.json["token"].as_str().unwrap().to_string()
        }
    };
    let host = mint(room.clone(), "host").await;
    let guest = mint(room.clone(), "guest").await;
    let elsewhere = mint(other.clone(), "host").await;
    let mute = format!("{base}/participants/p2/mute");
    let bearer = format!("Bearer {pete}");
    for (token, expected) in [
        (None, StatusCode::FORBIDDEN),
        (Some(guest.as_str()), StatusCode::FORBIDDEN),
        (Some(elsewhere.as_str()), StatusCode::FORBIDDEN),
        (Some("not-a-token"), StatusCode::FORBIDDEN),
        (Some(host.as_str()), StatusCode::OK),
    ] {
        let mut headers = vec![(AUTH_HEADER, bearer.as_str())];
        if let Some(token) = token {
            headers.push(("x-fcp-join", token));
        }
        let res = app.send(Method::POST, &mute, &headers, None).await;
        assert_eq!(res.status, expected, "{token:?}: {}", res.text);
    }
    assert_eq!(
        seen.lock().unwrap().len(),
        reached + 1,
        "only the host token's reached it"
    );
    // A room in no meeting of the tenant is not found.
    let none = app
        .send_as(&olga, Method::GET, "/api/v1/me/conferences/live/8999", None)
        .await;
    assert_eq!(none.status, StatusCode::NOT_FOUND, "{}", none.text);

    // Anyone of the tenant raises their own hand, named by their call; a
    // host's controls stay a host's.
    let before = seen.lock().unwrap().len();
    let res = app
        .send_as(
            &pete,
            Method::POST,
            &format!("{base}/hand"),
            Some(json!({ "call_id": " call-1 ", "raised": true })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.text);
    assert_eq!(res.json["path"], "/conferences/8402/hand");
    let last = seen.lock().unwrap().last().cloned().unwrap();
    assert_eq!(last.method, "POST");
    assert_eq!(
        last.body,
        json!({ "call_id": "call-1", "raised": true }),
        "the call trimmed"
    );
    let res = app
        .send_as(
            &pete,
            Method::POST,
            &format!("{base}/hand"),
            Some(json!({ "call_id": "", "raised": true })),
        )
        .await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST, "{}", res.text);
    let res = app
        .send_as(&pete, Method::DELETE, &format!("{base}/hands"), None)
        .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN, "{}", res.text);
    assert_eq!(seen.lock().unwrap().len(), before + 1);
    app.finish().await;
}

/// A webinar (`docs/WEBINARS.md` §2.1): its kind, panelist PIN and
/// panelists — users of the tenant, kept by id — set by an administrator
/// or the owner; the roles only a webinar has refused in a meeting's
/// token; a panelist minting a panelist token for themselves and no other.
#[tokio::test]
async fn a_webinar_has_panelists_who_mint_their_own_tokens() {
    let app = require_app!().with_join_secret();
    let olga = app.create_user_with_role("olga", "end_user").await;
    let pam = app.create_user_with_role("pam", "end_user").await;
    app.create_user_with_role("pete", "end_user").await;
    let id = new_meeting(
        &app,
        app.tenant_a,
        "8401",
        json!({ "kind": "webinar", "owner_id": "olga", "host_pin": "9999",
                "panelist_pin": "5555", "panelists": ["pam"] }),
    )
    .await;
    let path = format!("/api/v1/conferences/meetings/{id}");
    let got = app.admin(Method::GET, &path, None).await;
    assert_eq!(got.json["kind"], "webinar", "{}", got.text);
    assert_eq!(got.json["panelist_pin"], "5555");
    assert_eq!(got.json["panelists"], json!([pam]), "kept by id");

    // A PIN that is another role's, a stranger, an unknown kind: refused.
    for body in [
        json!({ "panelist_pin": "9999" }),
        json!({ "panelists": ["nobody"] }),
        json!({ "kind": "townhall" }),
    ] {
        let res = app.admin(Method::PATCH, &path, Some(body.clone())).await;
        assert_eq!(res.status, StatusCode::BAD_REQUEST, "{body}: {}", res.text);
    }
    // By id as well as by name.
    let both = app
        .admin(
            Method::PATCH,
            &path,
            Some(json!({ "panelists": [pam.clone(), "olga"] })),
        )
        .await;
    assert_eq!(both.status, StatusCode::OK, "{}", both.text);
    assert_eq!(both.json["panelists"], json!([pam, olga]));

    // The owner may change them too.
    let olga_s = session(&app, "olga").await;
    let mine = format!("/api/v1/me/conferences/{id}");
    let res = app
        .send_as(
            &olga_s,
            Method::PATCH,
            &mine,
            Some(json!({ "panelists": ["pam"], "panelist_pin": null })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.text);
    assert_eq!(res.json["panelists"], json!([pam]));
    assert_eq!(res.json["panelist_pin"], Value::Null);

    // A panelist mints a panelist token, under their own name, whatever
    // they ask for; a host token is not theirs to mint.
    let pam_s = session(&app, "pam").await;
    let minted = app
        .send_as(&pam_s, Method::POST, &format!("{mine}/join-tokens"), None)
        .await;
    assert_eq!(minted.status, StatusCode::CREATED, "{}", minted.text);
    assert_eq!(minted.json["role"], "panelist", "{}", minted.text);
    assert_eq!(minted.json["display_name"], "pam");
    let host = app
        .send_as(
            &pam_s,
            Method::POST,
            &format!("{mine}/join-tokens"),
            Some(json!({ "role": "host" })),
        )
        .await;
    assert_eq!(host.status, StatusCode::FORBIDDEN, "{}", host.text);
    // Anyone else in the tenant sees no such conference.
    let pete_s = session(&app, "pete").await;
    let res = app
        .send_as(&pete_s, Method::POST, &format!("{mine}/join-tokens"), None)
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND, "{}", res.text);

    // A meeting has no panelists or attendees to mint for.
    let meeting = new_meeting(&app, app.tenant_a, "8402", json!({})).await;
    for role in ["panelist", "attendee"] {
        let res = app
            .admin(
                Method::POST,
                &format!("/api/v1/conferences/meetings/{meeting}/join-tokens"),
                Some(json!({ "role": role })),
            )
            .await;
        assert_eq!(res.status, StatusCode::BAD_REQUEST, "{role}: {}", res.text);
    }
    let attendee = app
        .admin(
            Method::POST,
            &format!("{path}/join-tokens"),
            Some(json!({ "role": "attendee" })),
        )
        .await;
    assert_eq!(attendee.status, StatusCode::CREATED, "{}", attendee.text);
    assert_eq!(attendee.json["role"], "attendee");
    app.finish().await;
}
