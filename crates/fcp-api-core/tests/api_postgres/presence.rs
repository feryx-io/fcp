//! Presence a person sets (`docs/PRESENCE.md`): `/me/presence` and a
//! user's own statuses; DND one switch both ways; what others see with
//! signing out, calls and idleness laid over the chosen status, announced
//! as `presence.user_changed` and shown in the directory; an
//! administrator's read.

use super::*;
use fcp_core::CredentialStore;
use fcp_events::{EventFilter, EventStream, EventSubscriber, PlatformEvent, PresenceEvent};

/// Signed in as a new end user; their id and token.
async fn signed_in(app: &App, username: &str) -> (String, String) {
    let id = app.create_user_with_role(username, "end_user").await;
    let login = app.login(username, PASSWORD).await;
    assert_eq!(login.status, StatusCode::OK, "{}", login.text);
    (id, login.json["token"].as_str().unwrap().to_string())
}

fn people(app: &App) -> &fcp_api_core::people_presence::PeoplePresence {
    app.state.people_presence.as_ref().expect("people presence")
}

/// The next `presence.user_changed` for `user_id`, as (state, label).
async fn next_change(stream: &mut EventStream, user_id: &str) -> (String, Option<String>) {
    loop {
        let event = tokio::time::timeout(Duration::from_secs(5), stream.next())
            .await
            .expect("a presence change")
            .expect("the stream");
        if let PlatformEvent::Presence(PresenceEvent::UserChanged {
            user_id: who,
            state,
            label,
            ..
        }) = event
        {
            if who == user_id {
                return (state, label);
            }
        }
    }
}

async fn shown(app: &App, token: &str) -> Value {
    let r = app
        .send_as(token, Method::GET, "/api/v1/me/presence", None)
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    r.json
}

async fn put(app: &App, token: &str, body: Value) -> Response {
    app.send_as(token, Method::PUT, "/api/v1/me/presence", Some(body))
        .await
}

async fn dnd(app: &App, token: &str) -> bool {
    let r = app
        .send_as(token, Method::GET, "/api/v1/me/calling", None)
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    r.json["dnd"].as_bool().unwrap()
}

#[tokio::test]
async fn a_user_keeps_their_own_statuses_and_picks_one() {
    let Some(app) = App::build(DeploymentMode::default(), None).await else {
        return;
    };
    let (_, jo) = signed_in(&app, "presence-jo").await;
    let (_, sam) = signed_in(&app, "presence-sam").await;

    let fresh = shown(&app, &jo).await;
    assert_eq!(fresh["status"], "available");
    assert_eq!(fresh["statuses"], json!([]));

    let mut ids = Vec::new();
    for (label, state) in [("WFH", "available"), ("At lunch", "away"), ("Focus", "dnd")] {
        let r = app
            .send_as(
                &jo,
                Method::POST,
                "/api/v1/me/presence/statuses",
                Some(json!({"label": format!(" {label} "), "state": state})),
            )
            .await;
        assert_eq!(r.status, StatusCode::CREATED, "{}", r.text);
        assert_eq!(r.json["label"], label, "trimmed");
        ids.push(r.json["id"].as_str().unwrap().to_string());
    }
    let unnamed = app
        .send_as(
            &jo,
            Method::POST,
            "/api/v1/me/presence/statuses",
            Some(json!({"label": "  ", "state": "away"})),
        )
        .await;
    assert_eq!(unnamed.status, StatusCode::BAD_REQUEST, "{}", unnamed.text);
    let unknown = app
        .send_as(
            &jo,
            Method::POST,
            "/api/v1/me/presence/statuses",
            Some(json!({"label": "Odd", "state": "on_call"})),
        )
        .await;
    assert!(unknown.status.is_client_error(), "{}", unknown.text);

    // Reordered: every id, once.
    let reordered = app
        .send_as(
            &jo,
            Method::PUT,
            "/api/v1/me/presence/statuses/order",
            Some(json!({"ids": [&ids[2], &ids[0], &ids[1]]})),
        )
        .await;
    assert_eq!(reordered.status, StatusCode::OK, "{}", reordered.text);
    let labels = |v: &Value| -> Vec<String> {
        v["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["label"].as_str().unwrap().to_string())
            .collect()
    };
    let listed = app
        .send_as(&jo, Method::GET, "/api/v1/me/presence/statuses", None)
        .await;
    assert_eq!(labels(&listed.body), ["Focus", "WFH", "At lunch"]);
    let partial = app
        .send_as(
            &jo,
            Method::PUT,
            "/api/v1/me/presence/statuses/order",
            Some(json!({"ids": [&ids[0]]})),
        )
        .await;
    assert_eq!(partial.status, StatusCode::BAD_REQUEST, "{}", partial.text);

    // Picked with a note and a time it clears at.
    let until = chrono::Utc::now() + chrono::Duration::hours(1);
    let r = put(
        &app,
        &jo,
        json!({"status": &ids[1], "note": "back at 2", "until": until.to_rfc3339()}),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert_eq!(r.json["status"], ids[1].as_str());
    assert_eq!(r.json["status_state"], "away");
    assert_eq!(r.json["note"], "back at 2");
    assert!(r.json["until"].is_string());

    // A note alone changes only the note.
    let r = put(&app, &jo, json!({"note": "back at 3"})).await;
    assert_eq!(r.json["status"], ids[1].as_str());
    assert_eq!(r.json["note"], "back at 3");

    for bad in [
        json!({"status": "nonsense"}),
        json!({"status": "away", "until": "2001-01-01T00:00:00Z"}),
        json!({"status": "away", "until": (chrono::Utc::now() + chrono::Duration::days(8)).to_rfc3339()}),
        json!({"status": "away", "note": "x".repeat(101)}),
    ] {
        let r = put(&app, &jo, bad.clone()).await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{bad}: {}", r.text);
    }
    let theirs = put(&app, &sam, json!({"status": &ids[0]})).await;
    assert_eq!(theirs.status, StatusCode::BAD_REQUEST, "not Sam's status");
    let theirs = app
        .send_as(
            &sam,
            Method::DELETE,
            &format!("/api/v1/me/presence/statuses/{}", ids[0]),
            None,
        )
        .await;
    assert_eq!(theirs.status, StatusCode::NOT_FOUND, "{}", theirs.text);

    // Renamed, then deleted while in use: back to available.
    let r = app
        .send_as(
            &jo,
            Method::PUT,
            &format!("/api/v1/me/presence/statuses/{}", ids[1]),
            Some(json!({"label": "Lunch"})),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert_eq!(r.json["label"], "Lunch");
    let mine = shown(&app, &jo).await;
    assert_eq!(
        mine["presence"]["state"], "offline",
        "nothing of Jo's is registered: others see offline, with no label"
    );
    let r = app
        .send_as(
            &jo,
            Method::DELETE,
            &format!("/api/v1/me/presence/statuses/{}", ids[1]),
            None,
        )
        .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT, "{}", r.text);
    let after = shown(&app, &jo).await;
    assert_eq!(after["status"], "available");
    assert_eq!(after["statuses"].as_array().unwrap().len(), 2);

    // An administrator reads it; only they may.
    put(&app, &jo, json!({"status": &ids[0], "note": "home"})).await;
    let r = app
        .admin(Method::GET, "/api/v1/users/presence-jo/presence", None)
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert_eq!(r.json["status_label"], "WFH");
    assert_eq!(r.json["note"], "home");
    let r = app
        .send_as(
            &sam,
            Method::GET,
            "/api/v1/users/presence-jo/presence",
            None,
        )
        .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN, "{}", r.text);
}

#[tokio::test]
async fn do_not_disturb_is_one_switch() {
    let Some(app) = App::build(DeploymentMode::default(), None).await else {
        return;
    };
    let (_, jo) = signed_in(&app, "presence-dnd").await;
    let focus = app
        .send_as(
            &jo,
            Method::POST,
            "/api/v1/me/presence/statuses",
            Some(json!({"label": "Focus", "state": "dnd"})),
        )
        .await
        .json["id"]
        .as_str()
        .unwrap()
        .to_string();

    put(&app, &jo, json!({"status": &focus})).await;
    assert!(dnd(&app, &jo).await, "a dnd status turns DND on");
    put(&app, &jo, json!({"status": "busy"})).await;
    assert!(!dnd(&app, &jo).await, "any other turns it off");

    put(&app, &jo, json!({"status": "dnd"})).await;
    assert!(dnd(&app, &jo).await);
    // DND turned off elsewhere (`*79`, the app's switch) ends the status.
    let r = app
        .send_as(
            &jo,
            Method::PUT,
            "/api/v1/me/calling",
            Some(json!({"dnd": false})),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let event = PlatformEvent::User(fcp_events::UserEvent::CallingChanged {
        user_id: shown_user_id(&app, "presence-dnd").await,
        username: "presence-dnd".into(),
        tenant_id: app.tenant_a,
        timestamp: SystemTime::now(),
    });
    people(&app).on_event(&event).await;
    assert_eq!(shown(&app, &jo).await["status"], "available");

    // A status turned to dnd while in use turns DND on.
    put(&app, &jo, json!({"status": &focus})).await;
    put(&app, &jo, json!({"status": "available"})).await;
    let wfh = app
        .send_as(
            &jo,
            Method::POST,
            "/api/v1/me/presence/statuses",
            Some(json!({"label": "Heads down", "state": "available"})),
        )
        .await
        .json["id"]
        .as_str()
        .unwrap()
        .to_string();
    put(&app, &jo, json!({"status": &wfh})).await;
    assert!(!dnd(&app, &jo).await);
    app.send_as(
        &jo,
        Method::PUT,
        &format!("/api/v1/me/presence/statuses/{wfh}"),
        Some(json!({"state": "dnd"})),
    )
    .await;
    assert!(dnd(&app, &jo).await);
}

async fn shown_user_id(app: &App, username: &str) -> String {
    app.storage
        .get_user_in_tenant(username, &app.tenant_a)
        .await
        .unwrap()
        .unwrap()
        .id
        .to_string()
}

#[tokio::test]
async fn others_see_signing_out_calls_and_idleness_over_the_chosen_status() {
    let Some(app) = App::build(DeploymentMode::default(), None).await else {
        return;
    };
    let (jo_id, jo) = signed_in(&app, "presence-pat").await;
    let (_, sam) = signed_in(&app, "presence-lee").await;
    let mut events = app
        .event_bus
        .subscribe(EventFilter::new())
        .await
        .expect("subscribe");
    let pat_in_directory = |r: &Response| -> Value {
        r.body["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["username"] == "presence-pat")
            .cloned()
            .expect("Pat listed")
    };

    // Nothing registered: offline to others, whatever was chosen.
    let wfh = app
        .send_as(
            &jo,
            Method::POST,
            "/api/v1/me/presence/statuses",
            Some(json!({"label": "WFH", "state": "available"})),
        )
        .await
        .json["id"]
        .as_str()
        .unwrap()
        .to_string();
    put(&app, &jo, json!({"status": &wfh})).await;
    let r = app
        .send_as(&sam, Method::GET, "/api/v1/me/directory", None)
        .await;
    assert_eq!(pat_in_directory(&r)["presence"], "offline");

    // Two phones register; one leaves and Pat is still there.
    app.seed_registration(
        app.tenant_a,
        &format!("presence-pat@{REALM}"),
        "sip:presence-pat@192.0.2.1:5060",
    )
    .await;
    app.seed_registration(
        app.tenant_a,
        &format!("presence-pat@{REALM}"),
        "sip:presence-pat@192.0.2.2:5060",
    )
    .await;
    let registered = |contact: &str| {
        PlatformEvent::Registration(fcp_events::RegistrationEvent::Registered {
            aor: format!("sip:presence-pat@{REALM}"),
            contact: contact.into(),
            tenant_id: app.tenant_a,
            expires_secs: 3600,
            user_agent: None,
            timestamp: SystemTime::now(),
            instance_id: None,
        })
    };
    people(&app)
        .on_event(&registered("sip:presence-pat@192.0.2.1:5060"))
        .await;
    assert_eq!(
        next_change(&mut events, &jo_id).await,
        ("available".into(), Some("WFH".into()))
    );
    app.storage
        .delete_registration(
            &format!("presence-pat@{REALM}"),
            "sip:presence-pat@192.0.2.2:5060",
        )
        .await
        .unwrap();
    people(&app)
        .on_event(&PlatformEvent::Registration(
            fcp_events::RegistrationEvent::Unregistered {
                aor: format!("sip:presence-pat@{REALM}"),
                contact: "sip:presence-pat@192.0.2.2:5060".into(),
                tenant_id: app.tenant_a,
                reason: "explicit".into(),
                timestamp: SystemTime::now(),
            },
        ))
        .await;
    let r = app
        .send_as(&sam, Method::GET, "/api/v1/me/directory", None)
        .await;
    let pat = pat_in_directory(&r);
    assert_eq!(pat["presence"], "available", "one phone of two left");
    assert_eq!(pat["presence_label"], "WFH");

    // A call rings Pat, is answered, and ends: back to what Pat chose.
    let call = CallId::new();
    people(&app)
        .on_event(&PlatformEvent::Call(fcp_events::CallEvent::Initiated {
            call_id: call,
            tenant_id: app.tenant_a,
            from: format!("sip:presence-lee@{REALM}"),
            to: format!("sip:presence-pat@{REALM}"),
            direction: fcp_events::CallDirection::Inbound,
            timestamp: SystemTime::now(),
            trunk_name: None,
            trunk_id: None,
            node_id: None,
        }))
        .await;
    assert_eq!(next_change(&mut events, &jo_id).await.0, "ringing");
    people(&app)
        .on_event(&PlatformEvent::Presence(PresenceEvent::UserCall {
            user_id: jo_id.clone(),
            call_id: call.to_string(),
            on_call: true,
            tenant_id: app.tenant_a,
            timestamp: SystemTime::now(),
        }))
        .await;
    assert_eq!(
        next_change(&mut events, &jo_id).await,
        ("on_call".into(), Some("WFH".into())),
        "the label kept beside the call"
    );
    people(&app)
        .on_event(&PlatformEvent::Presence(PresenceEvent::UserCall {
            user_id: jo_id.clone(),
            call_id: call.to_string(),
            on_call: false,
            tenant_id: app.tenant_a,
            timestamp: SystemTime::now(),
        }))
        .await;
    assert_eq!(
        next_change(&mut events, &jo_id).await,
        ("available".into(), Some("WFH".into())),
        "what Pat chose, not a reset"
    );

    // Idle from an app: away until back.
    let r = put(&app, &jo, json!({"idle": true})).await;
    assert_eq!(r.json["presence"]["state"], "away");
    assert_eq!(r.json["presence"]["idle"], true);
    assert_eq!(next_change(&mut events, &jo_id).await.0, "away");
    let r = put(&app, &jo, json!({"idle": false})).await;
    assert_eq!(r.json["presence"]["state"], "available");

    // Busy chosen: idle does not stand in for it.
    put(&app, &jo, json!({"status": "busy"})).await;
    let r = put(&app, &jo, json!({"idle": true})).await;
    assert_eq!(r.json["presence"]["state"], "busy");
}

#[tokio::test]
async fn a_status_clears_itself_when_its_time_comes() {
    let Some(app) = App::build(DeploymentMode::default(), None).await else {
        return;
    };
    let (jo_id, jo) = signed_in(&app, "presence-until").await;
    put(&app, &jo, json!({"status": "dnd", "until": (chrono::Utc::now() + chrono::Duration::minutes(30)).to_rfc3339()})).await;
    assert!(dnd(&app, &jo).await);

    // Its time passed (written past, as a clock would leave it).
    let store = app.dyn_storage.presence_status_store().unwrap();
    let user: UserId = jo_id.parse().unwrap();
    let mut chosen = store.get_chosen_status(&user).await.unwrap().unwrap();
    chosen.until = Some(SystemTime::now() - Duration::from_secs(1));
    store.put_chosen_status(&chosen).await.unwrap();
    people(&app).chosen_changed(&app.tenant_a, &user).await;
    people(&app).tick().await;

    assert_eq!(shown(&app, &jo).await["status"], "available");
    assert!(
        !dnd(&app, &jo).await,
        "DND went with the status that set it"
    );
    assert!(store.get_chosen_status(&user).await.unwrap().is_none());
}

/// A user made after the tenant's presence was read is known at once, not
/// when the tenant is next read: before v0.25.627 their idleness was
/// dropped and they were shown as what they chose, signed in or not.
#[tokio::test]
async fn a_user_made_after_the_tenant_was_read_has_presence_at_once() {
    let Some(app) = App::build(DeploymentMode::default(), None).await else {
        return;
    };
    let (_, first) = signed_in(&app, "presence-first").await;
    // Reads the tenant's people.
    assert_eq!(shown(&app, &first).await["presence"]["state"], "offline");

    let (_, late) = signed_in(&app, "presence-late").await;
    assert_eq!(
        shown(&app, &late).await["presence"]["state"],
        "offline",
        "known, with nothing registered; not unknown and shown as chosen"
    );
    let late_id: fcp_core::UserId = shown_user_id(&app, "presence-late").await.parse().unwrap();
    assert!(people(&app).of(&app.tenant_a, &late_id).await.is_some());
}
