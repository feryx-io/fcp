//! Every route, every tenant, every role (docs/DEEP_TESTING.md D3).
//!
//! Two probes generated from `route_permissions::ROUTES`, so a route added
//! to the table is probed without anyone remembering to:
//!
//! - **Another tenant's ids.** Tenant B is seeded with one of everything;
//!   tenant A's administrator then calls every route naming an object with
//!   B's ids. Nothing may answer 2xx, nothing may answer 5xx, and B's
//!   objects read the same afterwards. The list routes are called with the
//!   platform's tenant header naming B, and must not show B's ids.
//! - **Every role.** A user of each built-in role calls every gated route
//!   with ids that name nothing. The permission table says who passes: a
//!   role without the permission gets 403, a role with it gets anything
//!   but 401 or 403.
//!
//! Both collect every disagreement and fail once, listing them all. Their
//! requests carry `route_log::PROBE_HEADER`, so they do not count toward
//! `scripts/check-routes-called.py`: a route is called by a test of what
//! it does.

use super::*;
use fcp_api_core::route_permissions::{Gate, ROUTES};
use fcp_core::permissions::{roles_have_permission, UserRole};
use futures_util::FutureExt;
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// The routes `fcp-admin` binds on top of the API's router; this harness
/// serves only the API's, so they answer the router's 404 here.
fn bound_elsewhere(path: &str) -> bool {
    path.starts_with("/cluster/") || path.starts_with("/system/services")
}

/// `template` with each `:param` replaced by `fill(prefix)`, where
/// `prefix` is the template up to the parameter (`/groups/:id/members`
/// for `:username` in `/groups/:id/members/:username`). `None` when a
/// parameter has no value.
fn concrete(template: &str, fill: impl Fn(&str) -> Option<String>) -> Option<String> {
    let mut out = String::new();
    let mut so_far = String::new();
    for seg in template.split('/').filter(|s| !s.is_empty()) {
        if seg.starts_with(':') {
            let value = fill(&so_far)?;
            out.push('/');
            out.push_str(&urlencode(&value));
        } else {
            out.push('/');
            out.push_str(seg);
        }
        so_far.push('/');
        so_far.push_str(seg);
    }
    Some(out)
}

fn urlencode(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
            c => {
                let mut buf = [0u8; 4];
                c.encode_utf8(&mut buf)
                    .bytes()
                    .map(|b| format!("%{b:02X}"))
                    .collect()
            }
        })
        .collect()
}

fn method_of(m: &str) -> Method {
    m.parse().expect("a method")
}

/// Send one request, catching a panic in the harness's contract check so
/// the probe can report it with the rest.
async fn probe(
    app: &App,
    token: &str,
    method: Method,
    path: &str,
    extra: &[(&str, &str)],
    body: Option<Value>,
) -> Result<Response, String> {
    let header = format!("Bearer {token}");
    let mut headers = vec![
        (AUTH_HEADER, header.as_str()),
        (fcp_api_core::route_log::PROBE_HEADER, "1"),
    ];
    headers.extend_from_slice(extra);
    std::panic::AssertUnwindSafe(app.send(method, path, &headers, body))
        .catch_unwind()
        .await
        .map_err(|p| {
            p.downcast_ref::<String>()
                .cloned()
                .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_else(|| "panic".into())
        })
}

/// Tenant B's objects, by the template prefix that names them.
struct Seeded {
    /// `/devices` → B's device id, `/groups/:id/members` → B's member …
    ids: HashMap<&'static str, String>,
    /// Every id and name of B's the probe must never see.
    secrets: BTreeSet<String>,
}

/// Create `body` at `path` in tenant B as the platform; the new object's
/// field `field`.
async fn make_in_b(app: &App, path: &str, body: Value, field: &str) -> String {
    let b = app.tenant_b.to_string();
    let res = app
        .send(
            Method::POST,
            path,
            &[(AUTH_HEADER, BEARER), ("x-fcp-tenant-id", b.as_str())],
            Some(body),
        )
        .await;
    assert!(
        res.status.is_success(),
        "seed {path} in tenant B: {} {}",
        res.status,
        res.text
    );
    let mut v = &res.json;
    for part in field.split('.') {
        v = &v[part];
    }
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        other => panic!("seed {path}: no {field} in {}", other),
    }
}

/// One of everything in tenant B.
async fn seed_tenant_b(app: &App) -> Seeded {
    let b = app.tenant_b;
    let mut ids: HashMap<&'static str, String> = HashMap::new();

    let user = "bravo-user";
    let user_id = make_in_b(
        app,
        "/api/v1/users",
        json!({"username": user, "password": PASSWORD, "role": "end_user",
               "extension": "2901", "tenant_id": b}),
        "id",
    )
    .await;
    ids.insert("/users", user.into());
    ids.insert("/users/by-id", user_id.clone());
    ids.insert("/me/executives", user_id.clone());

    let group = make_in_b(app, "/api/v1/groups", json!({"name": "Bravo group"}), "id").await;
    let b_hdr = b.to_string();
    let added = app
        .send(
            Method::POST,
            &format!("/api/v1/groups/{group}/members"),
            &[(AUTH_HEADER, BEARER), ("x-fcp-tenant-id", b_hdr.as_str())],
            Some(json!({"username": user})),
        )
        .await;
    assert!(added.status.is_success(), "group member: {}", added.text);
    ids.insert("/groups", group);
    ids.insert("/groups/:id/members", user.into());

    let location = make_in_b(
        app,
        "/api/v1/locations",
        json!({"name": "Bravo site", "address_line1": "1 Bravo Way", "city": "Troy",
               "state": "NY", "postal_code": "12180", "country": "US",
               "e911_callback_number": "+15185550999"}),
        "id",
    )
    .await;
    ids.insert("/locations", location.clone());
    ids.insert(
        "/emergency/networks",
        make_in_b(
            app,
            "/api/v1/emergency/networks",
            json!({"cidr": "10.77.0.0/24", "location_id": location, "name": "Bravo LAN"}),
            "id",
        )
        .await,
    );

    let device = make_in_b(
        app,
        "/api/v1/devices",
        json!({"name": "Bravo phone", "device_type": "desk_phone", "tenant_id": b,
               "mac_address": "00:15:65:B0:B0:01", "owner_user_id": user_id, "model": "T54W"}),
        "id",
    )
    .await;
    ids.insert("/devices", device.clone());

    // An allow and a capture of B's device (docs/CALL_DIAGNOSTICS.md §3).
    {
        use fcp_core::device_capture::DeviceCaptureStore;
        use fcp_core::device_capture::{AllowSubject, CaptureAllow, DeviceCapture};
        let device_id: fcp_core::DeviceId = device.parse().expect("a device id");
        let allow = CaptureAllow {
            id: uuid::Uuid::new_v4(),
            tenant_id: b,
            subject: AllowSubject::Device { device_id },
            until: std::time::SystemTime::now() + std::time::Duration::from_secs(86_400),
            reason: Some("bravo-capture-reason".into()),
            created_by: "seed".into(),
            created_at: std::time::SystemTime::now(),
        };
        app.storage.put_capture_allow(&allow).await.expect("allow");
        ids.insert("/captures/allows", format!("allow-{}", allow.id.simple()));
        let id = uuid::Uuid::new_v4();
        let capture = DeviceCapture {
            id,
            tenant_id: b,
            device_id,
            user_id: user_id.parse().expect("a user id"),
            call_id: None,
            size: 4,
            sha256: "00".repeat(32),
            file_key: DeviceCapture::key_for(&b, &id),
            subject_keys: Vec::new(),
            uploaded_at: std::time::SystemTime::now(),
            expires_at: std::time::SystemTime::now() + std::time::Duration::from_secs(86_400),
        };
        app.storage
            .put_device_capture(&capture)
            .await
            .expect("capture");
        ids.insert("/captures", format!("cap-{}", id.simple()));
    }

    let trunk = make_in_b(
        app,
        "/api/v1/trunks",
        json!({"name": "bravo-trunk", "hosts": ["sip.bravo.example.net"],
               "transport_protocol": "udp"}),
        "id",
    )
    .await;
    ids.insert("/trunks", trunk.clone());

    let number = make_in_b(
        app,
        "/api/v1/phone-numbers",
        json!({"e164": "+15550009901", "tenant_id": b}),
        "id",
    )
    .await;
    ids.insert("/phone-numbers", number.clone());
    let number2 = make_in_b(
        app,
        "/api/v1/phone-numbers",
        json!({"e164": "+15550009902", "tenant_id": b}),
        "id",
    )
    .await;
    let binding = make_in_b(
            app,
            "/api/v1/line-bindings",
            json!({"phone_number_id": number2, "owner_user_id": user_id,
                   "binding_type": "user", "tenant_id": b,
                   "devices": [{"device_id": device, "priority": 1, "line_appearance": 1, "shared_line": false, "enabled": true}]}),
            "id",
        )
        .await;
    ids.insert("/line-bindings", binding.clone());

    ids.insert(
        "/extensions",
        make_in_b(
            app,
            "/api/v1/extensions",
            json!({"number": "2999", "target": {"type": "user", "id": user_id}}),
            "number",
        )
        .await,
    );
    ids.insert(
        "/voicemail/mailboxes",
        make_in_b(
            app,
            "/api/v1/voicemail/mailboxes",
            json!({"pin": "4321", "user_id": user_id, "tenant_id": b,
                   "line_binding_id": binding}),
            "id",
        )
        .await,
    );
    ids.insert("/voicemail/mailboxes/:id/greetings", "unavailable".into());
    ids.insert(
        "/flows",
        make_in_b(
            app,
            "/api/v1/flows",
            hangup_flow("bravo-flow", "Bravo flow"),
            "id",
        )
        .await,
    );
    ids.insert("/flows/:id/versions", "1".into());
    ids.insert(
        "/prompts",
        make_in_b(
            app,
            "/api/v1/prompts",
            json!({"name": "bravo-welcome", "source_type": "file"}),
            "id",
        )
        .await,
    );
    ids.insert(
        "/config-templates",
        make_in_b(
            app,
            "/api/v1/config-templates",
            json!({"name": "Bravo template", "vendor": "yealink", "template_body": TEMPLATE}),
            "id",
        )
        .await,
    );
    ids.insert(
        "/classes-of-service",
        make_in_b(
            app,
            "/api/v1/classes-of-service",
            json!({"name": "Bravo class", "allowed": ["internal"]}),
            "id",
        )
        .await,
    );
    ids.insert(
        "/schedules",
        make_in_b(
            app,
            "/api/v1/schedules",
            json!({"name": "Bravo hours", "timezone": "America/New_York",
                   "weekly": [{"days": ["Mon"], "start": "09:00", "end": "17:00"}]}),
            "id",
        )
        .await,
    );
    ids.insert(
        "/parking/lots",
        make_in_b(
            app,
            "/api/v1/parking/lots",
            json!({"name": "Bravo lot", "first": "880", "last": "889"}),
            "id",
        )
        .await,
    );
    ids.insert(
        "/lcr-routes",
        make_in_b(
            app,
            "/api/v1/lcr-routes",
            json!({"prefix": "+49", "trunk_id": trunk, "cost_per_minute": 0.01}),
            "id",
        )
        .await,
    );
    ids.insert(
        "/routing/rules",
        make_in_b(
            app,
            "/api/v1/routing/rules",
            json!({"name": "bravo-rule", "priority": 10, "enabled": true,
                   "action": "route_to_trunk", "destination_pattern": "^\\+49",
                   "target_trunk_id": trunk}),
            "id",
        )
        .await,
    );
    ids.insert(
        "/queue-configs",
        make_in_b(
            app,
            "/api/v1/queue-configs",
            json!({"name": "bravo-queue", "extension": "2700", "strategy": "longest_idle"}),
            "id",
        )
        .await,
    );
    ids.insert(
        "/rate-plans",
        make_in_b(
            app,
            "/api/v1/rate-plans",
            json!({"name": "Bravo plan", "currency": "USD", "default_rate_per_minute": 0.02}),
            "id",
        )
        .await,
    );
    ids.insert(
        "/sites",
        make_in_b(app, "/api/v1/sites", json!({"name": "bravo-site"}), "id").await,
    );
    ids.insert(
        "/conferences/meetings",
        make_in_b(
            app,
            "/api/v1/conferences/meetings",
            json!({"room_number": "2800", "name": "Bravo room", "owner_id": user_id}),
            "id",
        )
        .await,
    );
    ids.insert(
        "/compliance/holds",
        make_in_b(
            app,
            "/api/v1/compliance/holds",
            json!({"matter": "Bravo v. Alpha", "reason": "litigation",
                   "subjects": [{"type": "user", "username": user}], "classes": ["cdr"]}),
            "id",
        )
        .await,
    );
    ids.insert(
        "/identity-providers",
        make_in_b(
            app,
            "/api/v1/identity-providers",
            json!({"name": "Bravo SSO", "client_secret": "bravo-secret",
                   "settings": {"issuer": "https://idp.bravo.example", "client_id": "bravo"}}),
            "id",
        )
        .await,
    );
    ids.insert(
        "/compliance/subject-requests",
        make_in_b(
            app,
            "/api/v1/compliance/subject-requests",
            json!({"kind": "export", "subjects": [{"type": "user", "username": user}]}),
            "id",
        )
        .await,
    );

    ids.insert(
        "/webhooks",
        make_in_b(
            app,
            "/api/v1/webhooks",
            json!({"url": "http://127.0.0.1:9/bravo"}),
            "webhook.id",
        )
        .await,
    );
    ids.insert("/webhooks/:id/deliveries", "no-such-delivery".into());
    let plan = ids["/rate-plans"].clone();
    ids.insert(
        "/rate-plans/:id/rates",
        make_in_b(
            app,
            &format!("/api/v1/rate-plans/{plan}/rates"),
            json!({"prefix": "1555", "rate_per_minute": 0.05}),
            "id",
        )
        .await,
    );
    ids.insert(
        "/users/:username/identities",
        ids["/identity-providers"].clone(),
    );
    ids.insert("/registrations/:aor", format!("sip:{user}@10.77.0.9:5060"));
    ids.insert("/presence/:aor/watchers", "no-such-subscription".into());
    let image = {
        let res = app
            .send_multipart(
                ADMIN_TOKEN,
                "/api/v1/firmware",
                &[
                    ("vendor", None, b"yealink".as_slice()),
                    ("version", None, b"96.86.0.90".as_slice()),
                    ("models", None, b"T5*".as_slice()),
                    (
                        "file",
                        Some(("T54W-96.86.0.90.rom", "application/octet-stream")),
                        b"bravo firmware".as_slice(),
                    ),
                ],
            )
            .await;
        assert_eq!(res.status, StatusCode::CREATED, "firmware: {}", res.text);
        res.json["id"].as_str().unwrap().to_string()
    };
    ids.insert("/firmware", image.clone());
    ids.insert(
        "/firmware/targets",
        make_in_b(
            app,
            "/api/v1/firmware/targets",
            json!({"vendor": "yealink", "model_pattern": "T5*", "image_id": image}),
            "id",
        )
        .await,
    );
    let uploaded = {
        let request = Request::builder()
            .method(Method::POST)
            .uri("/api/v1/branding/assets")
            .header(AUTH_HEADER, BEARER)
            .header("x-fcp-tenant-id", b_hdr.as_str())
            .header(header::CONTENT_TYPE, "image/png")
            .body(Body::from(tiny_png(8, 8, 0xb0)))
            .unwrap();
        let res = app.router.clone().oneshot(request).await.unwrap();
        let status = res.status();
        let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let v: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        assert!(status.is_success(), "branding asset: {status} {v}");
        v["sha256"].as_str().expect("sha256").to_string()
    };
    ids.insert("/branding/assets", uploaded);

    // What has no create route: written by phones, nodes and the call path.
    let mac = "00:15:65:B0:B0:02";
    fcp_core::provisioning::ProvisioningStore::note_pending_device(
        app.storage.as_ref(),
        &fcp_core::provisioning::PendingDevice {
            tenant_id: b,
            mac_address: mac.into(),
            first_seen: SystemTime::now(),
            last_seen: SystemTime::now(),
            address: Some("10.77.0.20".into()),
            user_agent: Some("Yealink SIP-T54W 96.86.0.70".into()),
            vendor: None,
            model: Some("T54W".into()),
            firmware: None,
            fetches: 1,
        },
    )
    .await
    .expect("pending phone");
    ids.insert("/provisioning/pending", mac.into());
    let hoteling = fcp_core::HotelingSession::start(
        b,
        device.parse().unwrap(),
        user_id.parse().unwrap(),
        Duration::from_secs(3600),
    );
    let hoteling_id = hoteling.id.to_string();
    fcp_core::hoteling::HotelingStore::start_hoteling_session(app.storage.as_ref(), hoteling)
        .await
        .expect("hoteling session");
    ids.insert("/hoteling/sessions", hoteling_id);
    let message = fcp_core::VoicemailMessage::new(
        ids["/voicemail/mailboxes"].clone(),
        "+15550004444".into(),
        5,
        "/vm/bravo.wav".into(),
    );
    {
        use fcp_core::storage::VoicemailStore;
        app.storage.put_message(message.clone()).await.unwrap();
    }
    ids.insert("/voicemail/messages", message.id.clone());
    let token_id = fcp_core::ApiTokenId::new();
    fcp_core::storage::ApiTokenStore::create_api_token(
        app.dyn_storage
            .api_token_store()
            .expect("token store")
            .as_ref(),
        fcp_core::storage::ApiToken {
            id: token_id,
            name: "bravo-token".into(),
            token_hash: fcp_core::storage::ApiToken::hash_secret("fcp_bravo_secret"),
            prefix: "fcp_brav".into(),
            user_id: user_id.clone(),
            tenant_id: b,
            permissions: vec!["device_read".into()],
            expires_at: None,
            created_at: SystemTime::now(),
            last_used_at: None,
            scope: Default::default(),
            revoked_at: None,
            revoked_by: None,
            previous_token_hash: None,
            previous_expires_at: None,
            rotated_at: None,
        },
    )
    .await
    .expect("token");
    ids.insert("/api-tokens", token_id.to_string());
    let emergency = {
        use fcp_core::e911::{EmergencyCallRecord, EmergencyKind, EmergencyStore};
        let call = CallId::new().to_string();
        app.emergency
            .insert_emergency_call(&EmergencyCallRecord {
                id: 0,
                tenant_id: b,
                call_id: call.clone(),
                sip_call_id: Some("bravo@10.77.0.9".into()),
                caller_uri: format!("sip:{user}@{REALM}"),
                caller_username: Some(user.into()),
                dialed_number: "911".into(),
                kind: EmergencyKind::Live,
                callback_number: Some("+15185550999".into()),
                location: None,
                trunk_name: None,
                trunk_id: None,
                outcome: "placed".into(),
                placed_at: SystemTime::now(),
                notified: false,
                sip_status: None,
                answered_at: None,
                ended_at: None,
                delivery: None,
                location_structured: None,
                geolocation_error: None,
            })
            .await
            .expect("emergency call");
        call
    };
    {
        use fcp_core::e911::notify::*;
        use fcp_core::e911::{EmergencyKind, EmergencyStore};
        let notice = EmergencyNotice {
            tenant_id: b,
            call_id: emergency.clone(),
            kind: EmergencyKind::Live,
            dialed: "911".into(),
            caller: format!("sip:{user}@{REALM}"),
            caller_name: Some(user.into()),
            callback_number: Some("+15185550999".into()),
            location_summary: Some("Bravo site: 1 Bravo Way".into()),
            location_source: None,
            client_address: None,
            trunk_name: None,
            location_id: None,
            placed_at: SystemTime::now(),
        };
        let row = EmergencyNotification::first(
            b,
            None,
            emergency.clone(),
            NotifyContact::new(ContactKind::Email, "guard@bravo.example"),
            notice,
        );
        app.emergency
            .insert_emergency_notification(&row)
            .await
            .expect("notification");
    }
    ids.insert("/emergency/calls", emergency);
    for (path, body) in [
        (
            format!("/api/v1/users/{user}/calling"),
            json!({"dnd": true}),
        ),
        (
            format!("/api/v1/presence/{user}@{REALM}/policy"),
            json!({"default_allow": false, "allow_list": ["sip:bravo-friend@bravo.example"], "block_list": []}),
        ),
    ] {
        let res = app
            .send(
                Method::PUT,
                &path,
                &[(AUTH_HEADER, BEARER), ("x-fcp-tenant-id", b_hdr.as_str())],
                Some(body),
            )
            .await;
        assert!(res.status.is_success(), "seed {path}: {}", res.text);
    }
    // B's live room, answered by the stand-in conference server.
    ids.insert("/conferences/live", "2800".into());
    ids.insert("/conferences/live/:room/video/pin", "p-bravo".into());
    ids.insert("/conferences/live/:room/video/spotlight", "p-bravo".into());
    ids.insert("/conferences/live/:room/participants", "p-bravo".into());
    ids.insert(
        "/conferences/live/:room/participants/:pid/video",
        "off".into(),
    );

    let (call_id, sip_call_id) = app.seed_cdr_full(b, Some(30_000), None).await;
    ids.insert("/cdrs", call_id.to_string());
    ids.insert("/flow-traces", call_id.to_string());
    ids.insert("/calls", call_id.to_string());
    // A slot of B's lot below; the calls in it are held by a call manager,
    // which this router has none of (503).
    ids.insert("/parking/calls", "880".to_string());
    app.seed_sip_trace(&call_id.to_string(), &sip_call_id, b, "inbound", "INVITE")
        .await;
    let recording = register_recording(app, b, "2800", "node-b", true).await;
    ids.insert("/recordings", recording.id.to_string());
    ids.insert("/tenants", b.to_string());

    // A configuration section of B's own (the platform writes them).
    fcp_core::config_store::ConfigStore::set_config(
        app.storage.as_ref(),
        "bravo-svc",
        "bravo-section",
        &b.to_string(),
        json!({"key": "bravo-config-secret"}),
        Some("seed"),
    )
    .await
    .expect("config");
    ids.insert("/config", "bravo-svc".into());
    ids.insert("/config/:service", "bravo-section".into());

    let aor = format!("{user}@{REALM}");
    ids.insert("/registrations", aor.clone());
    ids.insert("/presence", aor);

    // What would show B's data: its ids and names, not the platform's
    // firmware image or the made-up names filling a parameter — nor a
    // parking slot's three digits, which any timestamp may contain.
    let mut secrets: BTreeSet<String> = ids
        .iter()
        .filter(|(k, _)| {
            !matches!(
                **k,
                "/firmware"
                    | "/flows/:id/versions"
                    | "/voicemail/mailboxes/:id/greetings"
                    | "/webhooks/:id/deliveries"
                    | "/presence/:aor/watchers"
                    | "/conferences/live"
                    | "/conferences/live/:room/video/pin"
                    | "/conferences/live/:room/video/spotlight"
                    | "/conferences/live/:room/participants"
                    | "/conferences/live/:room/participants/:pid/video"
                    | "/parking/calls"
            )
        })
        .map(|(_, v)| v.clone())
        .collect();
    secrets.insert(b.to_string());
    secrets.insert(user_id);
    secrets.insert("sip:bravo-friend@bravo.example".into());
    secrets.insert("guard@bravo.example".into());
    secrets.insert("bravo-config-secret".into());
    Seeded { ids, secrets }
}

/// The routes the cross-tenant probe does not call, and why.
fn not_cross_tenant(method: &str, path: &str) -> Option<&'static str> {
    if path.starts_with("/me/") || path.starts_with("/auth/") {
        return Some("the caller's own; a /me route names the caller's objects");
    }
    if path.starts_with("/provisioning/redirection/") {
        return Some("a redirection service by name: the caller's own tenant's account");
    }
    if path == "/config-templates/library/:vendor" {
        return Some("the built-in starters by vendor, copied into the caller's tenant");
    }
    if path.starts_with("/queues/") {
        return Some("the runtime queue manager's queues by name; seeded only for tenant A");
    }
    if bound_elsewhere(path) {
        return Some("bound by fcp-admin, not this router");
    }
    let _ = method;
    None
}

/// A route that answers 2xx for any key because what it answers is the
/// caller's own tenant's.
enum PerTenant {
    /// A list under the key, filtered by the caller's tenant: B's key gives
    /// A an empty list (B has rows there, so empty proves the filter).
    FilteredList(&'static str),
    /// Kept per tenant by a key no tenant owns (an AOR): A's write lands in
    /// A's namespace, and B's view of the key is compared afterwards.
    Namespaced,
}

fn per_tenant_answer(template: &str) -> Option<PerTenant> {
    match template {
        "/emergency/calls/:call_id/notifications" => Some(PerTenant::FilteredList("data")),
        "/calls/:call_id/sip-messages" => Some(PerTenant::FilteredList("messages")),
        "/presence/:aor/policy" | "/presence/:aor/pres-rules" => Some(PerTenant::Namespaced),
        _ => None,
    }
}

/// docs/DEEP_TESTING.md D3: tenant A's administrator, with tenant B's ids,
/// reaches nothing of B's.
#[tokio::test]
async fn no_route_reaches_another_tenants_objects() {
    let (conference_url, upstream) = mock_conference_server().await;
    // Private webhook targets, so B's webhook needs no DNS.
    let Some(app) =
        App::build_with(DeploymentMode::default(), Some(conference_url), true, None).await
    else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping PostgreSQL API handler test");
        return;
    };
    let firmware = tempfile::tempdir().unwrap();
    let app = app.with_firmware(firmware.path());
    let seeded = seed_tenant_b(&app).await;
    app.create_user_with_role("alpha-admin", "tenant_admin")
        .await;
    let jwt = session(&app, "alpha-admin").await;
    let b = app.tenant_b.to_string();
    let b_hdr = [("x-fcp-tenant-id", b.as_str())];

    let mut wrong: Vec<String> = Vec::new();
    let mut unresolved: Vec<String> = Vec::new();
    let mut called = 0usize;

    // The lists first, asked for B by the platform's header, while A's own
    // records (its audit log) hold nothing of B's.
    for (m, template, _) in ROUTES {
        if *m != "GET" || template.contains(':') || not_cross_tenant(m, template).is_some() {
            continue;
        }
        let url = format!("/api/v1{template}");
        called += 1;
        match probe(&app, &jwt, Method::GET, &url, &b_hdr, None).await {
            Err(panic) => wrong.push(format!("{m} {template}: {panic}")),
            Ok(res) => {
                if res.status.is_server_error() && res.status != StatusCode::SERVICE_UNAVAILABLE {
                    wrong.push(format!("{m} {template} answered {}", res.status));
                }
                let leaked: Vec<&String> = seeded
                    .secrets
                    .iter()
                    .filter(|s| res.text.contains(s.as_str()))
                    .collect();
                if !leaked.is_empty() {
                    wrong.push(format!(
                        "{m} {template} with B's tenant header shows B's {leaked:?}"
                    ));
                }
            }
        }
    }

    // B's objects as B sees them, to compare after the probe.
    let mut before: BTreeMap<String, Value> = BTreeMap::new();
    for (m, template, _) in ROUTES {
        if *m != "GET" || not_cross_tenant(m, template).is_some() || !template.contains(':') {
            continue;
        }
        let Some(path) = concrete(template, |p| seeded.ids.get(p).cloned()) else {
            continue;
        };
        let url = format!("/api/v1{path}");
        match probe(&app, ADMIN_TOKEN, Method::GET, &url, &b_hdr, None).await {
            Err(panic) => wrong.push(format!("B's own GET {template}: {panic}")),
            Ok(res) => {
                let json = res
                    .headers
                    .get(header::CONTENT_TYPE)
                    .and_then(|c| c.to_str().ok())
                    .is_some_and(|c| c.starts_with("application/json"));
                if res.status.is_success() && json {
                    before.insert(url, res.body);
                }
            }
        }
    }

    for (m, template, _) in ROUTES {
        if !template.contains(':') || not_cross_tenant(m, template).is_some() {
            continue;
        }
        let method = method_of(m);
        let Some(path) = concrete(template, |p| seeded.ids.get(p).cloned()) else {
            unresolved.push(format!("{m} {template}"));
            continue;
        };
        let url = format!("/api/v1{path}");
        // A write carries B's own object, so a handler that checks the body
        // before the tenant is still reached.
        let body = match method {
            Method::PUT | Method::PATCH => {
                Some(before.get(&url).cloned().unwrap_or_else(|| json!({})))
            }
            Method::POST => Some(json!({})),
            _ => None,
        };
        called += 1;
        if template.starts_with("/conferences/live/") {
            // The conference server scopes its rooms by the tenant the API
            // names; the API must name the caller's, whatever the request
            // says.
            upstream.lock().unwrap().clear();
            let _ = probe(&app, &jwt, method, &url, &b_hdr, body).await;
            let forwarded: Vec<Option<String>> = upstream
                .lock()
                .unwrap()
                .iter()
                .map(|u| u.tenant.clone())
                .collect();
            let a = app.tenant_a.to_string();
            if forwarded.iter().any(|t| t.as_deref() != Some(a.as_str())) {
                wrong.push(format!(
                    "{m} {template} reached the conference server as {forwarded:?}, not tenant A"
                ));
            }
            continue;
        }
        let res = match probe(&app, &jwt, method, &url, &[], body.clone()).await {
            Ok(res) => res,
            Err(panic) => {
                wrong.push(format!("{m} {template}: {panic}"));
                continue;
            }
        };
        // What B's id in the path or B's object in the body brought along
        // is not a leak.
        let sent = body.as_ref().map(Value::to_string).unwrap_or_default();
        let leaked: Vec<&String> = seeded
            .secrets
            .iter()
            .filter(|s| {
                !url.contains(urlencode(s).as_str())
                    && !sent.contains(s.as_str())
                    && res.text.contains(s.as_str())
            })
            .collect();
        let fine = match res.status {
            // Not configured here: no call manager, no TTS.
            StatusCode::SERVICE_UNAVAILABLE => true,
            s if s.is_server_error() => false,
            s if s.is_success() => match per_tenant_answer(template) {
                Some(PerTenant::FilteredList(field)) => {
                    res.body[field].as_array().is_some_and(|a| a.is_empty())
                }
                // B's view of the same key is compared below.
                Some(PerTenant::Namespaced) => leaked.is_empty(),
                None => false,
            },
            _ => true,
        };
        if !fine {
            wrong.push(format!(
                "{m} {template} with B's ids answered {}: {}",
                res.status,
                res.text.chars().take(200).collect::<String>()
            ));
        }
    }

    // And B's objects are as they were.
    for (url, was) in &before {
        let now = probe(&app, ADMIN_TOKEN, Method::GET, url, &b_hdr, None)
            .await
            .map(|r| r.body)
            .unwrap_or(Value::Null);
        if &now != was {
            wrong.push(format!(
                "{url} changed under tenant A's probe:\n  was {was}\n  now {now}"
            ));
        }
    }

    assert!(called > 150, "probed only {called} routes");
    assert!(
        wrong.is_empty() && unresolved.is_empty(),
        "routes naming an object tenant B was not seeded with: {unresolved:#?}\n\
         {} of {called} probes reached tenant B or failed:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
    app.finish().await;
}

/// A platform administrator signed in works in the tenant they pick
/// (`docs/TENANT_SCOPE.md` §3.1): with tenant B's header, their session
/// answers every read as the static token does with it — B's lists show
/// B's objects, B's objects open — and without it, their own tenant. A
/// tenant administrator's header changes nothing (the probe above).
#[tokio::test]
async fn a_platform_administrator_works_in_the_tenant_picked() {
    let (conference_url, _upstream) = mock_conference_server().await;
    let Some(app) =
        App::build_with(DeploymentMode::default(), Some(conference_url), true, None).await
    else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping PostgreSQL API handler test");
        return;
    };
    let firmware = tempfile::tempdir().unwrap();
    let app = app.with_firmware(firmware.path());
    let seeded = seed_tenant_b(&app).await;
    app.create_user_with_role("platform-admin", "super_admin")
        .await;
    let jwt = session(&app, "platform-admin").await;
    let b = app.tenant_b.to_string();
    let b_hdr = [("x-fcp-tenant-id", b.as_str())];
    let shown = |text: &str| -> BTreeSet<String> {
        seeded
            .secrets
            .iter()
            .filter(|s| text.contains(s.as_str()))
            .cloned()
            .collect()
    };

    let mut wrong: Vec<String> = Vec::new();
    let mut compared = 0usize;
    for (m, template, _) in ROUTES {
        if *m != "GET" || not_cross_tenant(m, template).is_some() || bound_elsewhere(template) {
            continue;
        }
        let Some(path) = concrete(template, |p| seeded.ids.get(p).cloned()) else {
            continue;
        };
        let url = format!("/api/v1{path}");
        let (Ok(platform), Ok(session)) = (
            probe(&app, ADMIN_TOKEN, Method::GET, &url, &b_hdr, None).await,
            probe(&app, &jwt, Method::GET, &url, &b_hdr, None).await,
        ) else {
            wrong.push(format!("GET {template}: a probe panicked"));
            continue;
        };
        compared += 1;
        if platform.status != session.status {
            wrong.push(format!(
                "GET {template}: the static token answered {}, the session {}: {}",
                platform.status,
                session.status,
                session.text.chars().take(200).collect::<String>()
            ));
        } else if shown(&platform.text) != shown(&session.text) {
            wrong.push(format!(
                "GET {template}: the static token shows {:?} of B's, the session {:?}",
                shown(&platform.text),
                shown(&session.text)
            ));
        }
    }
    assert!(compared > 100, "compared only {compared} routes");
    assert!(
        wrong.is_empty(),
        "{} of {compared} reads differ between the static token and a platform \
         administrator's session in tenant B:\n{}",
        wrong.len(),
        wrong.join("\n")
    );

    // Made in the tenant picked; without a pick, in the session's own.
    let made = app
        .send(
            Method::POST,
            "/api/v1/schedules",
            &[(AUTH_HEADER, format!("Bearer {jwt}").as_str()), b_hdr[0]],
            Some(json!({"name": "Picked hours", "timezone": "UTC"})),
        )
        .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.text);
    assert_eq!(made.json["tenant_id"], b.as_str(), "{}", made.text);
    let own = app
        .send(
            Method::GET,
            "/api/v1/schedules",
            &[(AUTH_HEADER, format!("Bearer {jwt}").as_str())],
            None,
        )
        .await;
    assert!(!own.text.contains("Picked hours"), "{}", own.text);
    let unknown = app
        .send(
            Method::GET,
            "/api/v1/schedules",
            &[
                (AUTH_HEADER, format!("Bearer {jwt}").as_str()),
                ("x-fcp-tenant-id", &fcp_core::TenantId::new().to_string()),
            ],
            None,
        )
        .await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND, "{}", unknown.text);
    // The default tenant, which has no row until something is set on it,
    // is pickable all the same.
    let default = app
        .send(
            Method::GET,
            "/api/v1/schedules",
            &[
                (AUTH_HEADER, format!("Bearer {jwt}").as_str()),
                ("x-fcp-tenant-id", &fcp_core::TenantId::nil().to_string()),
            ],
            None,
        )
        .await;
    assert_eq!(default.status, StatusCode::OK, "{}", default.text);

    app.finish().await;
}

/// The built-in roles a session can hold, by their wire names.
const ROLES: &[(&str, UserRole)] = &[
    ("super_admin", UserRole::SuperAdmin),
    ("tenant_admin", UserRole::TenantAdmin),
    ("telecom_admin", UserRole::TelecomAdmin),
    ("user_admin", UserRole::UserAdmin),
    ("department_admin", UserRole::DepartmentAdmin),
    ("read_only", UserRole::ReadOnly),
    ("end_user", UserRole::EndUser),
];

/// Routes whose handler asks more of a caller than the table does, and
/// what: the role matrix expects these handlers' own 403 for a role the
/// table lets through.
fn handler_refuses(path: &str, role: UserRole) -> Option<&'static str> {
    match (path, role) {
        ("/tenants/:id/deletion", r) if !r.is_platform_admin() => {
            Some("a tenant's deletion is followed by the platform administrator")
        }
        ("/users/:username/purge", r) if !r.is_platform_admin() => Some(
            "purging before the grace period ends is a platform administrator's (docs/USERS.md)",
        ),
        _ => None,
    }
}

/// docs/DEEP_TESTING.md D3: each role reaches what the permission table
/// gives it and is refused the rest, in each deployment mode.
#[tokio::test]
async fn every_role_reaches_what_the_table_gives_it_in_an_enterprise() {
    role_matrix(DeploymentMode::Enterprise).await;
}

#[tokio::test]
async fn every_role_reaches_what_the_table_gives_it_at_a_service_provider() {
    role_matrix(DeploymentMode::ServiceProvider).await;
}

#[tokio::test]
async fn every_role_reaches_what_the_table_gives_it_at_a_carrier() {
    role_matrix(DeploymentMode::Carrier).await;
}

async fn role_matrix(mode: DeploymentMode) {
    let app = require_app!(mode);
    let mut wrong: Vec<String> = Vec::new();
    let mut called = 0usize;

    for (name, role) in ROLES {
        let username = format!("role-{}", name.replace('_', "-"));
        app.create_user_with_role(&username, name).await;
        let jwt = session(&app, &username).await;
        for (m, template, gate) in ROUTES {
            if *gate == Gate::Open || bound_elsewhere(template) {
                continue;
            }
            let allowed = match gate {
                Gate::Open => true,
                Gate::AnyAdmin => *role != UserRole::EndUser,
                Gate::Needs(p) => roles_have_permission(&[*role], *p, mode),
            };
            // Ids that name nothing: the gate decides before the handler
            // looks, and nothing is changed.
            let path = concrete(template, |_| Some("no-such-thing-0d3".into()))
                .expect("every parameter filled");
            let url = format!("/api/v1{path}");
            let method = method_of(m);
            let body =
                matches!(method, Method::POST | Method::PUT | Method::PATCH).then(|| json!({}));
            called += 1;
            let res = match probe(&app, &jwt, method, &url, &[], body.clone()).await {
                Ok(res) => res,
                Err(panic) => {
                    wrong.push(format!("{name}: {m} {template}: {panic}"));
                    continue;
                }
            };
            let refused = matches!(res.status, StatusCode::FORBIDDEN | StatusCode::UNAUTHORIZED);
            if allowed && refused {
                if handler_refuses(template, *role).is_none() {
                    wrong.push(format!(
                        "{name}: {m} {template} is the role's by the table but answered {}: {}",
                        res.status,
                        res.text.chars().take(160).collect::<String>()
                    ));
                }
            } else if !allowed && res.status != StatusCode::FORBIDDEN {
                wrong.push(format!(
                    "{name}: {m} {template} is not the role's but answered {}",
                    res.status
                ));
            } else if res.status.is_server_error() && res.status != StatusCode::SERVICE_UNAVAILABLE
            {
                wrong.push(format!(
                    "{name}: {m} {template} answered {}: {}",
                    res.status,
                    res.text.chars().take(160).collect::<String>()
                ));
            }
        }
    }

    assert!(called > 1500, "called only {called}");
    assert!(
        wrong.is_empty(),
        "{mode:?}: {} of {called} role checks disagree with the table:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
    app.finish().await;
}
