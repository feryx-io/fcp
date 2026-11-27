//! Voicemail — mailboxes, messages, greetings — and prompts, route by
//! route (docs/DEEP_TESTING.md D3b). Speech comes from a stand-in TTS
//! service on loopback, spoken to as the node speaks to `fcp-tts-server`.

use super::*;
use fcp_core::storage::VoicemailStore;

/// A TTS service that answers every text with a short WAV, and one
/// containing "unspeakable" with a 500. What it was asked to say is kept.
async fn stand_in_tts() -> (String, Arc<std::sync::Mutex<Vec<String>>>) {
    use axum::routing::post;
    let said: Arc<std::sync::Mutex<Vec<String>>> = Arc::default();
    let heard = Arc::clone(&said);
    let router = Router::new().route(
        "/v1/synthesize",
        post(move |axum::Json(body): axum::Json<Value>| {
            let heard = Arc::clone(&heard);
            async move {
                let text = body["text"].as_str().unwrap_or_default().to_string();
                heard.lock().unwrap().push(text.clone());
                if text.contains("unspeakable") {
                    return (StatusCode::INTERNAL_SERVER_ERROR, Vec::new());
                }
                (StatusCode::OK, wav_bytes(800))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    (format!("http://127.0.0.1:{port}"), said)
}

impl App {
    /// The app speaking through the stand-in TTS at `url`.
    async fn with_tts(mut self, url: &str) -> Self {
        let tts = fcp_media::tts::TtsService::new(fcp_media::tts::TtsConfig {
            provider: fcp_media::tts::TtsProviderConfig::Remote {
                url: url.to_string(),
                auth_token: "tts-token".to_string(),
                provider: None,
                tenant_id: None,
            },
            cache_dir: self._dir.path().join("tts-cache"),
            cache_max_size_mb: 10,
            default_language: "en-US".to_string(),
            default_voice: None,
        })
        .await
        .expect("TTS service");
        self.state = self.state.clone().with_tts_service(Arc::new(tts));
        self.router = create_router(self.state.clone(), &test_api_config());
        self
    }

    /// A mailbox for `username` (made here) on a number of its own: the
    /// user's id and the mailbox's.
    async fn user_with_mailbox(&self, username: &str, e164: &str) -> (String, String) {
        let user = self.create_user_with_role(username, "end_user").await;
        let number = self
            .admin(
                Method::POST,
                "/api/v1/phone-numbers",
                Some(json!({ "e164": e164, "tenant_id": self.tenant_a })),
            )
            .await;
        assert_eq!(number.status, StatusCode::CREATED, "{}", number.text);
        let binding = self
            .admin(
                Method::POST,
                "/api/v1/line-bindings",
                Some(
                    json!({ "phone_number_id": number.json["id"], "owner_user_id": user,
                             "binding_type": "user", "tenant_id": self.tenant_a }),
                ),
            )
            .await;
        assert_eq!(binding.status, StatusCode::CREATED, "{}", binding.text);
        let mailbox = self
            .admin(
                Method::POST,
                "/api/v1/voicemail/mailboxes",
                Some(json!({ "tenant_id": self.tenant_a, "user_id": user,
                             "line_binding_id": binding.json["id"], "pin": "4321" })),
            )
            .await;
        assert_eq!(mailbox.status, StatusCode::CREATED, "{}", mailbox.text);
        (user, mailbox.json["id"].as_str().unwrap().to_string())
    }

    /// A message left in `mailbox`, its audio on disk.
    async fn leave_message(&self, mailbox: &str, caller: &str) -> fcp_core::VoicemailMessage {
        let path = self._dir.path().join(format!("{}.wav", Uuid::new_v4()));
        std::fs::write(&path, wav_bytes(1600)).unwrap();
        let message = fcp_core::VoicemailMessage::new(mailbox.to_string(), caller.into(), 2, path);
        self.storage.put_message(message.clone()).await.unwrap();
        message
    }
}

/// The counts the last `voicemail.mwi_updated` for `mailbox` carried.
async fn announced(events: &mut fcp_events::EventStream, mailbox: &str) -> (u32, u32) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(left, events.next()).await {
            Ok(Some(PlatformEvent::Voicemail(fcp_events::VoicemailEvent::MwiUpdated {
                mailbox_id,
                new_messages,
                old_messages,
                ..
            }))) if mailbox_id == mailbox => return (new_messages, old_messages),
            Ok(Some(_)) => continue,
            _ => panic!("no voicemail.mwi_updated for {mailbox}"),
        }
    }
}

/// An administrator lists the tenant's mailboxes, turns one on, reads its
/// messages and marks them; each mark tells message waiting the counts.
/// Another tenant's are not found.
#[tokio::test]
async fn an_administrator_reads_and_marks_a_mailboxs_messages() {
    let app = require_app!();
    let (_, mailbox) = app.user_with_mailbox("vera", "+15550010001").await;
    let (_, other) = app.user_with_mailbox("walt", "+15550010002").await;
    let first = app.leave_message(&mailbox, "+15550020001").await;
    let second = app.leave_message(&mailbox, "+15550020002").await;
    app.leave_message(&other, "+15550020003").await;

    let listed = app
        .admin(Method::GET, "/api/v1/voicemail/mailboxes", None)
        .await;
    assert_eq!(listed.status, StatusCode::OK, "{}", listed.text);
    assert_eq!(listed.json.as_array().unwrap().len(), 2, "{}", listed.text);
    let in_b = app
        .admin_in(
            app.tenant_b,
            Method::GET,
            "/api/v1/voicemail/mailboxes",
            None,
        )
        .await;
    assert_eq!(in_b.json, json!([]), "{}", in_b.text);

    let off = app
        .admin(
            Method::POST,
            &format!("/api/v1/voicemail/mailboxes/{mailbox}/disable"),
            None,
        )
        .await;
    assert_eq!(off.json["enabled"], false, "{}", off.text);
    let on = app
        .admin(
            Method::POST,
            &format!("/api/v1/voicemail/mailboxes/{mailbox}/enable"),
            None,
        )
        .await;
    assert_eq!(on.status, StatusCode::OK, "{}", on.text);
    assert_eq!(on.json["enabled"], true, "{}", on.text);

    let messages = app
        .admin(
            Method::GET,
            &format!("/api/v1/voicemail/mailboxes/{mailbox}/messages"),
            None,
        )
        .await;
    assert_eq!(messages.status, StatusCode::OK, "{}", messages.text);
    let ids: Vec<&str> = messages
        .json
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids.len(), 2, "{}", messages.text);
    assert!(ids.contains(&first.id.as_str()) && ids.contains(&second.id.as_str()));

    let path = format!("/api/v1/voicemail/messages/{}", first.id);
    let got = app.admin(Method::GET, &path, None).await;
    assert_eq!(got.status, StatusCode::OK, "{}", got.text);
    assert_eq!(got.json["caller"], "+15550020001");
    assert_eq!(got.json["status"], "new");

    let mut events = fcp_events::EventSubscriber::subscribe(
        app.event_bus.as_ref(),
        fcp_events::EventFilter::new(),
    )
    .await
    .unwrap();
    let heard = app
        .admin(Method::PUT, &path, Some(json!({ "status": "heard" })))
        .await;
    assert_eq!(heard.status, StatusCode::OK, "{}", heard.text);
    assert_eq!(heard.json["status"], "heard");
    assert_eq!(announced(&mut events, &mailbox).await, (1, 1));
    let saved = app
        .admin(
            Method::PUT,
            &format!("/api/v1/voicemail/messages/{}", second.id),
            Some(json!({ "status": "saved" })),
        )
        .await;
    assert_eq!(saved.status, StatusCode::OK, "{}", saved.text);
    assert_eq!(announced(&mut events, &mailbox).await, (0, 2));
    let bad = app
        .admin(Method::PUT, &path, Some(json!({ "status": "lost" })))
        .await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST, "{}", bad.text);
    let deleted = app.admin(Method::DELETE, &path, None).await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT, "{}", deleted.text);
    assert_eq!(announced(&mut events, &mailbox).await, (0, 1));

    // Tenant B reaches none of it.
    for (method, path, body) in [
        (
            Method::GET,
            format!("/api/v1/voicemail/mailboxes/{mailbox}/messages"),
            None,
        ),
        (
            Method::POST,
            format!("/api/v1/voicemail/mailboxes/{mailbox}/enable"),
            None,
        ),
        (
            Method::GET,
            format!("/api/v1/voicemail/messages/{}", second.id),
            None,
        ),
        (
            Method::PUT,
            format!("/api/v1/voicemail/messages/{}", second.id),
            Some(json!({ "status": "new" })),
        ),
    ] {
        let res = app
            .admin_in(app.tenant_b, method.clone(), &path, body)
            .await;
        assert_eq!(
            res.status,
            StatusCode::NOT_FOUND,
            "{method} {path}: {}",
            res.text
        );
    }
    app.finish().await;
}

/// A mailbox's PIN is four to eight digits and kept hashed, whoever sets
/// it; a caller at the phone still gets in with it.
#[tokio::test]
async fn a_mailbox_pin_is_checked_and_kept_hashed() {
    let app = require_app!();
    let (_, mailbox) = app.user_with_mailbox("pia", "+15550010011").await;
    let stored = app.storage.get_mailbox(&mailbox).await.unwrap().unwrap();
    assert!(stored.pin.starts_with("$2"), "hashed when made");
    assert!(stored.verify_pin("4321"));

    let jwt = session(&app, "pia").await;
    for pin in ["12", "12ab", "123456789", ""] {
        let res = app
            .send_as(
                &jwt,
                Method::PUT,
                "/api/v1/me/voicemail",
                Some(json!({ "pin": pin })),
            )
            .await;
        assert_eq!(res.status, StatusCode::BAD_REQUEST, "{pin:?}: {}", res.text);
        let res = app
            .admin(
                Method::PUT,
                &format!("/api/v1/voicemail/mailboxes/{mailbox}"),
                Some(json!({ "pin": pin })),
            )
            .await;
        assert_eq!(
            res.status,
            StatusCode::BAD_REQUEST,
            "admin {pin:?}: {}",
            res.text
        );
    }
    let set = app
        .send_as(
            &jwt,
            Method::PUT,
            "/api/v1/me/voicemail",
            Some(json!({ "pin": "86420", "email_notification": true,
                         "email_address": "pia@example.com", "attach_audio": true })),
        )
        .await;
    assert_eq!(set.status, StatusCode::OK, "{}", set.text);
    assert_eq!(set.json["email_notification"], true);
    assert_eq!(set.json["email_address"], "pia@example.com");
    assert!(set.json.get("pin").is_none(), "{}", set.text);
    let stored = app.storage.get_mailbox(&mailbox).await.unwrap().unwrap();
    assert!(stored.pin.starts_with("$2"));
    assert!(stored.verify_pin("86420"));
    assert!(!stored.verify_pin("4321"));
    // A mailbox written before PINs were hashed still opens with its own.
    let mut legacy = stored.clone();
    legacy.pin = "2468".to_string();
    app.storage.put_mailbox(legacy.clone()).await.unwrap();
    assert!(legacy.verify_pin("2468") && !legacy.verify_pin("2469"));
    app.finish().await;
}

/// The caller's own messages: marked, heard as audio and deleted, each
/// telling message waiting; someone else's is not found.
#[tokio::test]
async fn a_user_hears_marks_and_deletes_their_own_messages() {
    let app = require_app!();
    let (_, mailbox) = app.user_with_mailbox("ines", "+15550010021").await;
    let (_, theirs) = app.user_with_mailbox("joe", "+15550010022").await;
    let mine = app.leave_message(&mailbox, "+15550020021").await;
    let not_mine = app.leave_message(&theirs, "+15550020022").await;
    let jwt = session(&app, "ines").await;

    let audio = app
        .send_as(
            &jwt,
            Method::GET,
            &format!("/api/v1/me/voicemail/messages/{}/audio", mine.id),
            None,
        )
        .await;
    assert_eq!(audio.status, StatusCode::OK, "{}", audio.text);
    assert_eq!(
        audio.headers.get(header::CONTENT_TYPE).unwrap(),
        "audio/wav"
    );
    assert_eq!(audio.bytes.len(), wav_bytes(1600).len());

    let mut events = fcp_events::EventSubscriber::subscribe(
        app.event_bus.as_ref(),
        fcp_events::EventFilter::new(),
    )
    .await
    .unwrap();
    let path = format!("/api/v1/me/voicemail/messages/{}", mine.id);
    let saved = app
        .send_as(&jwt, Method::PUT, &path, Some(json!({ "status": "saved" })))
        .await;
    assert_eq!(saved.status, StatusCode::OK, "{}", saved.text);
    assert_eq!(saved.json["status"], "saved");
    assert_eq!(announced(&mut events, &mailbox).await, (0, 1));
    let deleting = app
        .send_as(
            &jwt,
            Method::PUT,
            &path,
            Some(json!({ "status": "deleted" })),
        )
        .await;
    assert_eq!(
        deleting.status,
        StatusCode::BAD_REQUEST,
        "{}",
        deleting.text
    );

    let other = format!("/api/v1/me/voicemail/messages/{}", not_mine.id);
    for (method, path, body) in [
        (
            Method::PUT,
            other.clone(),
            Some(json!({ "status": "heard" })),
        ),
        (Method::GET, format!("{other}/audio"), None),
        (Method::DELETE, other.clone(), None),
    ] {
        let res = app.send_as(&jwt, method.clone(), &path, body).await;
        assert_eq!(
            res.status,
            StatusCode::NOT_FOUND,
            "{method} {path}: {}",
            res.text
        );
    }
    assert!(app
        .storage
        .get_message(&not_mine.id)
        .await
        .unwrap()
        .is_some());

    let gone = app.send_as(&jwt, Method::DELETE, &path, None).await;
    assert_eq!(gone.status, StatusCode::NO_CONTENT, "{}", gone.text);
    assert_eq!(announced(&mut events, &mailbox).await, (0, 0));
    let again = app.send_as(&jwt, Method::DELETE, &path, None).await;
    assert_eq!(again.status, StatusCode::NOT_FOUND, "{}", again.text);
    app.finish().await;
}

/// A greeting spoken from text: kept on the mailbox, made the active one
/// and back; a temporary one ends when asked, within a year; without TTS,
/// or when it fails, the user is told so.
#[tokio::test]
async fn a_user_has_a_greeting_spoken_and_chooses_it() {
    let (url, said) = stand_in_tts().await;
    let Some(app) = App::build(DeploymentMode::default(), None).await else {
        return;
    };
    app.user_with_mailbox("gail", "+15550010031").await;
    let jwt = session(&app, "gail").await;
    let synth = "/api/v1/me/voicemail/greetings/synthesize";
    let body = |text: &str| json!({ "greeting_type": "unavailable", "text": text });

    let none = app
        .send_as(&jwt, Method::POST, synth, Some(body("Hi")))
        .await;
    assert_eq!(
        none.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        none.text
    );

    let app = app.with_tts(&url).await;
    let jwt = session(&app, "gail").await;
    let made = app
        .send_as(
            &jwt,
            Method::POST,
            synth,
            Some(body("Gail is away. Leave a message.")),
        )
        .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.text);
    assert_eq!(made.json["greeting_type"], "unavailable");
    assert!(said
        .lock()
        .unwrap()
        .contains(&"Gail is away. Leave a message.".to_string()));
    let audio = app
        .send_as(
            &jwt,
            Method::GET,
            "/api/v1/me/voicemail/greetings/unavailable/audio",
            None,
        )
        .await;
    assert_eq!(audio.status, StatusCode::OK, "{}", audio.text);
    assert_eq!(audio.bytes.len(), wav_bytes(800).len());

    let temporary = app
        .send_as(
            &jwt,
            Method::POST,
            synth,
            Some(json!({ "greeting_type": "temporary", "text": "Out today",
                         "expires_in_hours": 8 })),
        )
        .await;
    assert_eq!(temporary.status, StatusCode::CREATED, "{}", temporary.text);
    assert!(
        temporary.json["expires_at"].is_string(),
        "{}",
        temporary.text
    );
    for (hours, what) in [
        (json!(0), "zero"),
        (json!(24 * 366), "past a year"),
        (json!(u64::MAX), "overflow"),
    ] {
        let res = app
            .send_as(
                &jwt,
                Method::POST,
                synth,
                Some(
                    json!({ "greeting_type": "temporary", "text": "x", "expires_in_hours": hours }),
                ),
            )
            .await;
        assert_eq!(res.status, StatusCode::BAD_REQUEST, "{what}: {}", res.text);
    }
    for (bad, what) in [
        (body("   "), "blank"),
        (body(&"a".repeat(2001)), "too long"),
        (json!({ "greeting_type": "jingle", "text": "x" }), "kind"),
    ] {
        let res = app.send_as(&jwt, Method::POST, synth, Some(bad)).await;
        assert_eq!(res.status, StatusCode::BAD_REQUEST, "{what}: {}", res.text);
    }
    let failed = app
        .send_as(&jwt, Method::POST, synth, Some(body("unspeakable")))
        .await;
    assert_eq!(
        failed.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        failed.text
    );

    let active = |kind: &str| format!("/api/v1/me/voicemail/greetings/{kind}/active");
    let on = app
        .send_as(
            &jwt,
            Method::PUT,
            &active("unavailable"),
            Some(json!({ "active": true })),
        )
        .await;
    assert_eq!(on.status, StatusCode::OK, "{}", on.text);
    assert_eq!(on.json["is_active"], true);
    let listed = app
        .send_as(&jwt, Method::GET, "/api/v1/me/voicemail/greetings", None)
        .await;
    let is_active = |kind: &str| {
        listed
            .json
            .as_array()
            .unwrap()
            .iter()
            .find(|g| g["greeting_type"] == kind)
            .map(|g| g["is_active"].clone())
    };
    assert_eq!(
        is_active("unavailable"),
        Some(json!(true)),
        "{}",
        listed.text
    );
    let off = app
        .send_as(
            &jwt,
            Method::PUT,
            &active("unavailable"),
            Some(json!({ "active": false })),
        )
        .await;
    assert_eq!(off.json["is_active"], false, "{}", off.text);
    let missing = app
        .send_as(
            &jwt,
            Method::PUT,
            &active("busy"),
            Some(json!({ "active": true })),
        )
        .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND, "{}", missing.text);
    let unknown = app
        .send_as(
            &jwt,
            Method::PUT,
            &active("jingle"),
            Some(json!({ "active": true })),
        )
        .await;
    assert_eq!(unknown.status, StatusCode::BAD_REQUEST, "{}", unknown.text);
    app.finish().await;
}

/// A prompt is changed in its tenant; a spoken one's new words are
/// spoken before they are kept, and refused, unchanged, when they cannot
/// be; `synthesize` speaks it again.
#[tokio::test]
async fn a_prompt_is_changed_and_spoken_again() {
    let (url, said) = stand_in_tts().await;
    let Some(app) = App::build(DeploymentMode::default(), None).await else {
        return;
    };
    let make = |body: Value| {
        let app = &app;
        async move {
            let res = app.admin(Method::POST, "/api/v1/prompts", Some(body)).await;
            assert_eq!(res.status, StatusCode::CREATED, "{}", res.text);
            res.json["id"].as_str().unwrap().to_string()
        }
    };
    let file = make(json!({ "name": "hold", "source_type": "file", "category": "ivr" })).await;
    let spoken =
        make(json!({ "name": "welcome", "source_type": "tts", "tts_text": "Welcome" })).await;
    let file_path = format!("/api/v1/prompts/{file}");
    let spoken_path = format!("/api/v1/prompts/{spoken}");

    let changed = app
        .admin(
            Method::PUT,
            &file_path,
            Some(json!({ "description": "Hold loop", "category": "moh" })),
        )
        .await;
    assert_eq!(changed.status, StatusCode::OK, "{}", changed.text);
    assert_eq!(changed.json["description"], "Hold loop");
    assert_eq!(changed.json["category"], "moh");
    let bad = app
        .admin(
            Method::PUT,
            &file_path,
            Some(json!({ "category": "jingles" })),
        )
        .await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST, "{}", bad.text);
    let from_b = app
        .admin_in(
            app.tenant_b,
            Method::PUT,
            &file_path,
            Some(json!({ "description": "x" })),
        )
        .await;
    assert_eq!(from_b.status, StatusCode::NOT_FOUND, "{}", from_b.text);

    // No TTS: new words cannot be spoken, so nothing changes.
    let unspoken = app
        .admin(
            Method::PUT,
            &spoken_path,
            Some(json!({ "tts_text": "Hello" })),
        )
        .await;
    assert_eq!(
        unspoken.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        unspoken.text
    );
    let kept = app.admin(Method::GET, &spoken_path, None).await;
    assert_eq!(kept.json["tts_text"], "Welcome", "{}", kept.text);
    let no_tts = app
        .admin(Method::POST, &format!("{spoken_path}/synthesize"), None)
        .await;
    assert_eq!(
        no_tts.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        no_tts.text
    );

    let app = app.with_tts(&url).await;
    let synthesized = app
        .admin(Method::POST, &format!("{spoken_path}/synthesize"), None)
        .await;
    assert_eq!(synthesized.status, StatusCode::OK, "{}", synthesized.text);
    assert_eq!(synthesized.json["has_audio"], true, "{}", synthesized.text);
    assert_eq!(
        synthesized.json["file_size"].as_u64(),
        Some(wav_bytes(800).len() as u64)
    );
    let respoken = app
        .admin(
            Method::PUT,
            &spoken_path,
            Some(json!({ "tts_text": "Welcome to Acme", "tts_voice": "alto" })),
        )
        .await;
    assert_eq!(respoken.status, StatusCode::OK, "{}", respoken.text);
    assert_eq!(respoken.json["tts_text"], "Welcome to Acme");
    assert_eq!(respoken.json["has_audio"], true, "{}", respoken.text);
    assert!(said
        .lock()
        .unwrap()
        .contains(&"Welcome to Acme".to_string()));

    let failed = app
        .admin(
            Method::PUT,
            &spoken_path,
            Some(json!({ "tts_text": "unspeakable" })),
        )
        .await;
    assert_eq!(
        failed.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        failed.text
    );
    let kept = app.admin(Method::GET, &spoken_path, None).await;
    assert_eq!(kept.json["tts_text"], "Welcome to Acme", "{}", kept.text);
    assert_eq!(kept.json["has_audio"], true, "{}", kept.text);

    let nothing_to_say = app
        .admin(Method::POST, &format!("{file_path}/synthesize"), None)
        .await;
    assert_eq!(
        nothing_to_say.status,
        StatusCode::BAD_REQUEST,
        "{}",
        nothing_to_say.text
    );
    let theirs = app
        .admin_in(
            app.tenant_b,
            Method::POST,
            &format!("{spoken_path}/synthesize"),
            None,
        )
        .await;
    assert_eq!(theirs.status, StatusCode::NOT_FOUND, "{}", theirs.text);
    app.finish().await;
}

/// What the stand-in TTS has been asked to say, waited for: a renamed
/// user's greeting is made in the background.
async fn spoken(said: &Arc<std::sync::Mutex<Vec<String>>>, text: &str) -> bool {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        if said.lock().unwrap().iter().any(|t| t == text) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    false
}

/// #343: a mailbox whose owner has a first and last name greets callers by
/// them, spoken once and kept; it is spoken again when the name or the
/// choice changes and not otherwise; standard and the owner's own greeting
/// are choices; an owner without names is greeted by the standard
/// greeting, and recording their own greeting makes it the one heard; a
/// recorded name is spliced into the sentence.
#[tokio::test]
async fn a_mailbox_greets_callers_by_its_owners_name() {
    let (url, said) = stand_in_tts().await;
    let Some(app) = App::build(DeploymentMode::default(), None).await else {
        return;
    };
    let app = app.with_tts(&url).await;
    let sentence = |name: &str| {
        format!("Sorry, {name} is not available. Please leave a message after the tone.")
    };

    let nina = app.create_user_with_role("nina", "end_user").await;
    let named = app
        .admin(
            Method::PUT,
            "/api/v1/users/nina",
            Some(json!({ "first_name": "Nina", "last_name": "Park" })),
        )
        .await;
    assert_eq!(named.status, StatusCode::OK, "{}", named.text);
    assert_eq!(named.json["first_name"], "Nina");
    assert_eq!(
        named.json["display_name"], "Nina Park",
        "reads as the names"
    );
    let made = app
        .admin(
            Method::POST,
            "/api/v1/voicemail/mailboxes",
            Some(json!({ "tenant_id": app.tenant_a, "user_id": nina, "pin": "4321" })),
        )
        .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.text);
    assert_eq!(made.json["greeting_choice"], Value::Null);
    assert_eq!(made.json["greeting_name"], "full_name");
    assert_eq!(made.json["greeting_heard"], "personalized", "{}", made.text);
    assert!(spoken(&said, &sentence("Nina Park")).await);
    let mailbox = made.json["id"].as_str().unwrap().to_string();
    let path = format!("/api/v1/voicemail/mailboxes/{mailbox}");
    let audio = app
        .admin(
            Method::GET,
            &format!("{path}/greetings/personalized/audio"),
            None,
        )
        .await;
    assert_eq!(audio.status, StatusCode::OK, "{}", audio.text);
    assert_eq!(&audio.bytes[..4], b"RIFF");
    // A personalized greeting is made, never uploaded.
    let upload = app
        .send_multipart(
            ADMIN_TOKEN,
            &format!("{path}/greetings"),
            &[
                ("greeting_type", None, b"personalized"),
                ("audio", Some(("g.wav", "audio/wav")), &wav_bytes(800)),
            ],
        )
        .await;
    assert_eq!(upload.status, StatusCode::BAD_REQUEST, "{}", upload.text);

    // Nothing the greeting says changed: not spoken again.
    let asked = said.lock().unwrap().len();
    let same = app
        .admin(Method::PUT, &path, Some(json!({ "max_messages": 50 })))
        .await;
    assert_eq!(same.status, StatusCode::OK, "{}", same.text);
    app.admin(
        Method::PUT,
        "/api/v1/users/nina",
        Some(json!({ "job_title": "Engineer" })),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        said.lock().unwrap().len(),
        asked,
        "{:?}",
        said.lock().unwrap()
    );

    // A new name, from wherever: spoken again.
    app.admin(
        Method::PUT,
        "/api/v1/users/nina",
        Some(json!({ "last_name": "Lee" })),
    )
    .await;
    assert!(spoken(&said, &sentence("Nina Lee")).await);
    // The display name, when the mailbox says it; one that is only the
    // names is not kept.
    let by_display = app
        .admin(
            Method::PUT,
            &path,
            Some(json!({ "greeting_name": "display_name" })),
        )
        .await;
    assert_eq!(by_display.status, StatusCode::OK, "{}", by_display.text);
    assert_eq!(by_display.json["greeting_name"], "display_name");
    let jwt = session(&app, "nina").await;
    let profile = app
        .send_as(
            &jwt,
            Method::PUT,
            "/api/v1/me/profile",
            Some(json!({ "display_name": "Nina Lee" })),
        )
        .await;
    assert_eq!(profile.status, StatusCode::OK, "{}", profile.text);
    let stored =
        fcp_core::CredentialStore::get_user_in_tenant(app.storage.as_ref(), "nina", &app.tenant_a)
            .await
            .unwrap()
            .unwrap();
    assert_eq!(stored.display_name, None, "only the names: not kept");
    app.send_as(
        &jwt,
        Method::PUT,
        "/api/v1/me/profile",
        Some(json!({ "display_name": "Captain Lee" })),
    )
    .await;
    assert!(spoken(&said, &sentence("Captain Lee")).await);

    // Standard: the personalized greeting goes.
    let standard = app
        .send_as(
            &jwt,
            Method::PUT,
            "/api/v1/me/voicemail",
            Some(json!({ "greeting_choice": "standard" })),
        )
        .await;
    assert_eq!(standard.status, StatusCode::OK, "{}", standard.text);
    assert_eq!(standard.json["greeting_choice"], "standard");
    assert_eq!(standard.json["greeting_heard"], "standard");
    let greetings = app
        .admin(Method::GET, &format!("{path}/greetings"), None)
        .await;
    assert!(
        !greetings.text.contains("personalized"),
        "{}",
        greetings.text
    );
    let bad = app
        .send_as(
            &jwt,
            Method::PUT,
            "/api/v1/me/voicemail",
            Some(json!({ "greeting_choice": "jingle" })),
        )
        .await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST, "{}", bad.text);

    // A recorded name, spliced between the sentence's halves.
    let name_wav = {
        // A second of something said, at 8 kHz.
        let mut wav = wav_bytes(8000);
        for (i, chunk) in wav[44..].chunks_mut(2).enumerate() {
            chunk.copy_from_slice(&(((i % 16) as i16 - 8) * 2000).to_le_bytes());
        }
        wav
    };
    let recorded = app
        .send_multipart(
            &jwt,
            "/api/v1/me/voicemail/greetings",
            &[
                ("greeting_type", None, b"name"),
                ("audio", Some(("name.wav", "audio/wav")), &name_wav),
            ],
        )
        .await;
    assert_eq!(recorded.status, StatusCode::CREATED, "{}", recorded.text);
    let spliced = app
        .send_as(
            &jwt,
            Method::PUT,
            "/api/v1/me/voicemail",
            Some(json!({ "greeting_choice": "personalized", "greeting_name": "recorded" })),
        )
        .await;
    assert_eq!(
        spliced.json["greeting_heard"], "personalized",
        "{}",
        spliced.text
    );
    assert!(spoken(&said, "Sorry,").await);
    assert!(
        spoken(
            &said,
            "is not available. Please leave a message after the tone."
        )
        .await
    );
    let audio = app
        .send_as(
            &jwt,
            Method::GET,
            "/api/v1/me/voicemail/greetings/personalized/audio",
            None,
        )
        .await;
    assert_eq!(audio.status, StatusCode::OK, "{}", audio.text);
    // Two halves of 800 samples, two gaps of 1000 and the name's second.
    let samples = (audio.bytes.len() - 44) / 2;
    assert_eq!(samples, 800 + 1000 + 8000 + 1000 + 800, "{samples}");

    // An owner without names: the standard greeting, until they record
    // their own, which is then what callers hear.
    let omar = app.create_user_with_role("omar", "end_user").await;
    let plain = app
        .admin(
            Method::POST,
            "/api/v1/voicemail/mailboxes",
            Some(json!({ "tenant_id": app.tenant_a, "user_id": omar, "pin": "4321" })),
        )
        .await;
    assert_eq!(plain.json["greeting_heard"], "standard", "{}", plain.text);
    assert!(!said.lock().unwrap().iter().any(|t| t.contains("omar")));
    let omar_box = plain.json["id"].as_str().unwrap().to_string();
    let own = app
        .send_multipart(
            ADMIN_TOKEN,
            &format!("/api/v1/voicemail/mailboxes/{omar_box}/greetings"),
            &[
                ("greeting_type", None, b"unavailable"),
                ("audio", Some(("g.wav", "audio/wav")), &wav_bytes(800)),
            ],
        )
        .await;
    assert_eq!(own.status, StatusCode::CREATED, "{}", own.text);
    let after = app
        .admin(
            Method::GET,
            &format!("/api/v1/voicemail/mailboxes/{omar_box}"),
            None,
        )
        .await;
    assert_eq!(after.json["greeting_choice"], "own", "{}", after.text);
    assert_eq!(after.json["greeting_heard"], "own");
    app.finish().await;
}

/// `docs/TENANT_SPEECH.md` S2: a tenant's speech settings are kept, read
/// back, checked, held to their tenant, and taken back to the defaults.
#[tokio::test]
async fn a_tenants_speech_settings_are_kept_and_cleared() {
    let Some(app) = App::build(DeploymentMode::default(), None).await else {
        return;
    };
    let fresh = app
        .admin(Method::GET, "/api/v1/speech/settings", None)
        .await;
    assert_eq!(fresh.status, StatusCode::OK, "{}", fresh.text);
    assert_eq!(fresh.json["configured"], false);
    assert!(fresh.json["voice"].is_null());

    let put = app
        .admin(
            Method::PUT,
            "/api/v1/speech/settings",
            Some(json!({
                "provider": "elevenlabs",
                "voice": "Rachel",
                "language": "en-GB",
                "voices": { "es": "es_ES-davefx-medium" },
            })),
        )
        .await;
    assert_eq!(put.status, StatusCode::OK, "{}", put.text);
    let got = app
        .admin(Method::GET, "/api/v1/speech/settings", None)
        .await;
    assert_eq!(got.json["configured"], true);
    assert_eq!(got.json["provider"], "elevenlabs");
    assert_eq!(got.json["voice"], "Rachel");
    assert_eq!(got.json["language"], "en-GB");
    assert_eq!(got.json["voices"]["es"], "es_ES-davefx-medium");

    // Another tenant's are its own.
    let theirs = app
        .admin_in(app.tenant_b, Method::GET, "/api/v1/speech/settings", None)
        .await;
    assert_eq!(theirs.json["configured"], false, "{}", theirs.text);

    let bad = app
        .admin(
            Method::PUT,
            "/api/v1/speech/settings",
            Some(json!({ "voice": "Rachel; DROP TABLE" })),
        )
        .await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST, "{}", bad.text);

    let cleared = app
        .admin(Method::DELETE, "/api/v1/speech/settings", None)
        .await;
    assert_eq!(cleared.status, StatusCode::NO_CONTENT, "{}", cleared.text);
    let back = app
        .admin(Method::GET, "/api/v1/speech/settings", None)
        .await;
    assert_eq!(back.json["configured"], false);
    app.finish().await;
}

/// D8 finding 68: a spoken prompt whose file this admin does not hold (or
/// that was never made) is spoken for its preview, as a call would hear it;
/// with no TTS service the preview is refused, saying why.
#[tokio::test]
async fn a_spoken_prompt_with_no_file_here_is_previewed_by_tts() {
    let (url, said) = stand_in_tts().await;
    let Some(app) = App::build(DeploymentMode::default(), None).await else {
        return;
    };
    // Made with no TTS: no file.
    let made = app
        .admin(
            Method::POST,
            "/api/v1/prompts",
            Some(
                json!({ "name": "after-hours", "source_type": "tts", "tts_text": "We are closed" }),
            ),
        )
        .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.text);
    assert_eq!(made.json["has_audio"], false, "{}", made.text);
    let audio = format!(
        "/api/v1/prompts/{}/audio",
        made.json["id"].as_str().unwrap()
    );

    let refused = app.admin(Method::GET, &audio, None).await;
    assert_eq!(
        refused.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        refused.text
    );
    assert!(refused.text.contains("[tts]"), "{}", refused.text);

    let app = app.with_tts(&url).await;
    let played = app.admin(Method::GET, &audio, None).await;
    assert_eq!(played.status, StatusCode::OK, "{}", played.text);
    assert!(played.text.starts_with("RIFF"), "a WAV comes back");
    assert!(said.lock().unwrap().iter().any(|t| t == "We are closed"));

    // A file prompt with no file is still not found.
    let file = app
        .admin(
            Method::POST,
            "/api/v1/prompts",
            Some(json!({ "name": "hold", "source_type": "file", "category": "ivr" })),
        )
        .await;
    let missing = app
        .admin(
            Method::GET,
            &format!(
                "/api/v1/prompts/{}/audio",
                file.json["id"].as_str().unwrap()
            ),
            None,
        )
        .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND, "{}", missing.text);
    app.finish().await;
}

/// `docs/TENANT_SPEECH.md` S2b: the Speech page's providers and voices come
/// from the TTS server, and a preview speaks a choice not yet saved for the
/// tenant.
#[tokio::test]
async fn speech_providers_voices_and_a_preview_come_from_the_tts_server() {
    use axum::routing::{get, post};
    let bodies: Arc<std::sync::Mutex<Vec<Value>>> = Arc::default();
    let heard = Arc::clone(&bodies);
    let router = Router::new()
        .route(
            "/v1/providers",
            get(|| async {
                axum::Json(json!([
                    { "name": "piper", "is_default": true },
                    { "name": "elevenlabs", "is_default": false },
                ]))
            }),
        )
        .route(
            "/v1/voices",
            get(
                |axum::extract::Query(q): axum::extract::Query<
                    std::collections::HashMap<String, String>,
                >| async move {
                    let provider = q.get("provider").cloned().unwrap_or_else(|| "piper".into());
                    axum::Json(json!([{
                        "name": format!("{provider}-voice"),
                        "display_name": format!("A {provider} voice"),
                        "language": q.get("language").cloned().unwrap_or_default(),
                        "gender": null,
                        "metadata": {},
                    }]))
                },
            ),
        )
        .route(
            "/v1/synthesize",
            post(move |axum::Json(body): axum::Json<Value>| {
                let heard = Arc::clone(&heard);
                async move {
                    heard.lock().unwrap().push(body);
                    (StatusCode::OK, wav_bytes(800))
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    let Some(app) = App::build(DeploymentMode::default(), None).await else {
        return;
    };
    // No TTS service: none to choose from, and nothing to speak.
    let no_tts = app
        .admin(Method::GET, "/api/v1/speech/providers", None)
        .await;
    assert_eq!(no_tts.status, StatusCode::OK, "{}", no_tts.text);
    assert_eq!(
        no_tts.json.as_array().map(Vec::len),
        Some(0),
        "{}",
        no_tts.text
    );
    let no_preview = app
        .admin(
            Method::POST,
            "/api/v1/speech/preview",
            Some(json!({ "text": "hi" })),
        )
        .await;
    assert_eq!(
        no_preview.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        no_preview.text
    );
    let app = app.with_tts(&url).await;

    let providers = app
        .admin(Method::GET, "/api/v1/speech/providers", None)
        .await;
    assert_eq!(providers.status, StatusCode::OK, "{}", providers.text);
    assert_eq!(providers.json[0]["name"], "piper");
    assert_eq!(providers.json[0]["is_default"], true);
    assert_eq!(providers.json[1]["name"], "elevenlabs");

    let voices = app
        .admin(
            Method::GET,
            "/api/v1/speech/voices?provider=elevenlabs&language=fr-FR",
            None,
        )
        .await;
    assert_eq!(voices.status, StatusCode::OK, "{}", voices.text);
    assert_eq!(voices.json[0]["name"], "elevenlabs-voice");
    assert_eq!(voices.json[0]["language"], "fr-FR");

    let spoken = app
        .admin(
            Method::POST,
            "/api/v1/speech/preview",
            Some(json!({
                "text": "Bonjour",
                "provider": "elevenlabs",
                "voice": "elevenlabs-voice",
                "language": "fr-FR",
            })),
        )
        .await;
    assert_eq!(spoken.status, StatusCode::OK, "{}", spoken.text);
    assert!(spoken.bytes.starts_with(b"RIFF"), "a WAV");
    let asked = bodies.lock().unwrap().last().cloned().unwrap();
    assert_eq!(asked["provider"], "elevenlabs");
    assert_eq!(asked["voice"], "elevenlabs-voice");
    assert_eq!(asked["language"], "fr-FR");
    assert!(asked["tenant_id"].is_string(), "the tenant named: {asked}");

    let empty = app
        .admin(
            Method::POST,
            "/api/v1/speech/preview",
            Some(json!({ "text": " " })),
        )
        .await;
    assert_eq!(empty.status, StatusCode::BAD_REQUEST, "{}", empty.text);
    app.finish().await;
}

/// `docs/TENANT_SPEECH.md` S3: a tenant's own TTS account is made with its
/// key, which is never shown again; chosen in the settings; listed among
/// the providers; changed keeping its key; and deleted, taking the choice
/// of it with it.
#[tokio::test]
async fn a_tenants_speech_account_is_made_chosen_and_deleted() {
    use axum::routing::{get, post};
    let asked: Arc<std::sync::Mutex<Vec<Value>>> = Arc::default();
    let heard = Arc::clone(&asked);
    let router = Router::new()
        .route(
            "/v1/providers",
            get(|| async { axum::Json(json!([{ "name": "piper", "is_default": true }])) }),
        )
        .route(
            "/v1/synthesize",
            post(move |axum::Json(body): axum::Json<Value>| {
                let heard = Arc::clone(&heard);
                async move {
                    heard.lock().unwrap().push(body);
                    (StatusCode::OK, wav_bytes(800))
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    let Some(app) = App::build(DeploymentMode::default(), None).await else {
        return;
    };
    let app = app.with_tts(&url).await;
    let make = |body: Value| app.admin(Method::POST, "/api/v1/speech/accounts", Some(body));

    for (bad, why) in [
        (json!({ "name": "x", "kind": "openai" }), "no key"),
        (
            json!({ "name": "x", "kind": "azure", "secret": "k" }),
            "no region",
        ),
        (
            json!({ "name": "x", "kind": "aws_polly", "region": "us-east-1", "secret": "k" }),
            "no key id",
        ),
        (
            json!({ "name": "x", "kind": "elevenlabs", "secret": "k", "endpoint": "http://plain" }),
            "not https",
        ),
        (
            json!({ "name": " ", "kind": "openai", "secret": "k" }),
            "no name",
        ),
    ] {
        let r = make(bad).await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{why}: {}", r.text);
    }

    let made = make(json!({
        "name": "Our ElevenLabs",
        "kind": "elevenlabs",
        "model": "eleven_multilingual_v2",
        "secret": "xi-very-secret",
    }))
    .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.text);
    assert_eq!(made.json["has_key"], true);
    assert!(!made.text.contains("xi-very-secret"), "{}", made.text);
    let id = made.json["id"].as_str().unwrap().to_string();
    let provider = made.json["provider"].as_str().unwrap().to_string();
    assert_eq!(
        provider,
        format!("account:{}", id.strip_prefix("speech-").unwrap())
    );

    let again = make(json!({ "name": "our elevenlabs", "kind": "openai", "secret": "k" })).await;
    assert_eq!(again.status, StatusCode::CONFLICT, "{}", again.text);

    let listed = app
        .admin(Method::GET, "/api/v1/speech/accounts", None)
        .await;
    assert_eq!(listed.status, StatusCode::OK, "{}", listed.text);
    assert_eq!(
        listed.json.as_array().map(Vec::len),
        Some(1),
        "{}",
        listed.text
    );
    assert!(!listed.text.contains("xi-very-secret"));

    // Among the providers, after the TTS server's.
    let providers = app
        .admin(Method::GET, "/api/v1/speech/providers", None)
        .await;
    assert_eq!(providers.json[0]["name"], "piper", "{}", providers.text);
    assert_eq!(providers.json[1]["name"], provider.as_str());
    assert_eq!(providers.json[1]["label"], "Our ElevenLabs");
    assert_eq!(providers.json[1]["account"], true);

    // Chosen; an account the tenant does not have is refused.
    let chose = app
        .admin(
            Method::PUT,
            "/api/v1/speech/settings",
            Some(json!({ "provider": provider, "voice": "21m00Tcm4TlvDq8ikWAM" })),
        )
        .await;
    assert_eq!(chose.status, StatusCode::OK, "{}", chose.text);
    let stranger = app
        .admin(
            Method::PUT,
            "/api/v1/speech/settings",
            Some(json!({ "provider": format!("account:{}", uuid::Uuid::new_v4()) })),
        )
        .await;
    assert_eq!(
        stranger.status,
        StatusCode::BAD_REQUEST,
        "{}",
        stranger.text
    );

    // Tested: a sentence spoken through it, for this tenant.
    let path = format!("/api/v1/speech/accounts/{id}");
    let tested = app.admin(Method::POST, &format!("{path}/test"), None).await;
    assert_eq!(tested.status, StatusCode::OK, "{}", tested.text);
    assert!(tested.bytes.starts_with(b"RIFF"), "a WAV");
    let sent = asked.lock().unwrap().last().cloned().unwrap();
    assert_eq!(sent["provider"], provider.as_str(), "{sent}");
    assert!(sent["tenant_id"].is_string(), "{sent}");
    let missing = app
        .admin(
            Method::POST,
            &format!(
                "/api/v1/speech/accounts/speech-{}/test",
                uuid::Uuid::new_v4()
            ),
            None,
        )
        .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND, "{}", missing.text);

    // Changed without its key: the key is kept. Another vendor needs one.
    let renamed = app
        .admin(
            Method::PUT,
            &path,
            Some(json!({ "name": "Voices", "kind": "elevenlabs" })),
        )
        .await;
    assert_eq!(renamed.status, StatusCode::OK, "{}", renamed.text);
    assert_eq!(renamed.json["name"], "Voices");
    assert_eq!(renamed.json["has_key"], true);
    let switched = app
        .admin(
            Method::PUT,
            &path,
            Some(json!({ "name": "Voices", "kind": "openai" })),
        )
        .await;
    assert_eq!(
        switched.status,
        StatusCode::BAD_REQUEST,
        "{}",
        switched.text
    );

    // Deleted: the settings that chose it no longer do.
    let gone = app.admin(Method::DELETE, &path, None).await;
    assert_eq!(gone.status, StatusCode::NO_CONTENT, "{}", gone.text);
    let settings = app
        .admin(Method::GET, "/api/v1/speech/settings", None)
        .await;
    assert!(settings.json["provider"].is_null(), "{}", settings.text);
    assert!(settings.json["voice"].is_null(), "{}", settings.text);
    let twice = app.admin(Method::DELETE, &path, None).await;
    assert_eq!(twice.status, StatusCode::NOT_FOUND, "{}", twice.text);
    app.finish().await;
}

/// `docs/TENANT_SPEECH.md` S3: the platform holds a provider back from
/// tenants. A tenant's administrator no longer sees or may choose it; the
/// platform administrator may; the default cannot be held back.
#[tokio::test]
async fn the_platform_holds_a_provider_back_from_tenants() {
    use axum::routing::get;
    let router = Router::new().route(
        "/v1/providers",
        get(|| async {
            axum::Json(json!([
                { "name": "piper", "is_default": true },
                { "name": "elevenlabs", "is_default": false },
            ]))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    let Some(app) = App::build(DeploymentMode::default(), None).await else {
        return;
    };
    let app = app.with_tts(&url).await;
    app.create_user_with_role("speechta", "tenant_admin").await;
    let login = app.login("speechta", PASSWORD).await;
    let ta = login.json["token"].as_str().unwrap().to_string();

    let refused = app
        .send_as(
            &ta,
            Method::PUT,
            "/api/v1/system/speech",
            Some(json!({ "held_back": [] })),
        )
        .await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.text);
    let default = app
        .admin(
            Method::PUT,
            "/api/v1/system/speech",
            Some(json!({ "held_back": ["piper"] })),
        )
        .await;
    assert_eq!(default.status, StatusCode::BAD_REQUEST, "{}", default.text);

    let held = app
        .admin(
            Method::PUT,
            "/api/v1/system/speech",
            Some(json!({ "held_back": ["elevenlabs"] })),
        )
        .await;
    assert_eq!(held.status, StatusCode::OK, "{}", held.text);
    assert_eq!(held.json["providers"][1]["name"], "elevenlabs");
    assert_eq!(held.json["providers"][1]["offered"], false);
    assert_eq!(held.json["providers"][0]["offered"], true);
    let read = app.admin(Method::GET, "/api/v1/system/speech", None).await;
    assert_eq!(read.status, StatusCode::OK, "{}", read.text);
    assert_eq!(read.json["held_back"], json!(["elevenlabs"]));
    let not_theirs = app
        .send_as(&ta, Method::GET, "/api/v1/system/speech", None)
        .await;
    assert_eq!(
        not_theirs.status,
        StatusCode::FORBIDDEN,
        "{}",
        not_theirs.text
    );

    // The tenant's administrator: not listed, not choosable.
    let theirs = app
        .send_as(&ta, Method::GET, "/api/v1/speech/providers", None)
        .await;
    assert_eq!(theirs.status, StatusCode::OK, "{}", theirs.text);
    assert_eq!(
        theirs.json.as_array().map(Vec::len),
        Some(1),
        "{}",
        theirs.text
    );
    let chose = app
        .send_as(
            &ta,
            Method::PUT,
            "/api/v1/speech/settings",
            Some(json!({ "provider": "elevenlabs" })),
        )
        .await;
    assert_eq!(chose.status, StatusCode::FORBIDDEN, "{}", chose.text);

    // The platform's administrator still may.
    let all = app
        .admin(Method::GET, "/api/v1/speech/providers", None)
        .await;
    assert_eq!(all.json.as_array().map(Vec::len), Some(2), "{}", all.text);

    // Offered again.
    let back = app
        .admin(
            Method::PUT,
            "/api/v1/system/speech",
            Some(json!({ "held_back": [] })),
        )
        .await;
    assert_eq!(back.status, StatusCode::OK, "{}", back.text);
    let chose = app
        .send_as(
            &ta,
            Method::PUT,
            "/api/v1/speech/settings",
            Some(json!({ "provider": "elevenlabs" })),
        )
        .await;
    assert_eq!(chose.status, StatusCode::OK, "{}", chose.text);
    app.finish().await;
}
