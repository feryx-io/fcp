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
