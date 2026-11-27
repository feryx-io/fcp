//! Flows that listen, against the running binary
//! (`docs/DEVELOPER_SURFACE.md` phase 7). A menu's Gather takes digits or
//! speech: a caller who "says" sales (a tone the testkit's scripted
//! recognizer hears as words) is taken there, one who presses 2 by SIP
//! INFO goes to the other branch, and one who says nothing times out. The
//! sales branch starts the call's live transcript, and what the caller
//! says next reaches the events endpoint as `call.transcript`. A noise
//! recognised as no words does not hold the menu open. And a call runs
//! the version of its flow it started on (phase 7b). Skipped
//! without `FCP_TEST_DATABASE_URL`.

use axum::{extract::State, routing::post, Json, Router};
use fcp_core::flow_trace::{FlowTraceEventType, FlowTraceQuery, FlowTraceStore};
use fcp_core::storage::{LineBindingStore, PhoneNumberStore};
use fcp_testkit::media::allocate_rtp_range;
use fcp_testkit::smoke::loopback;
use fcp_testkit::stt::ScriptedStt;
use fcp_testkit::{free_port, Phone, Seed, Service, TestDatabase};
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_fcp-call-manager");
const REALM: &str = "e2e.local";
const SOON: Duration = Duration::from_secs(5);
const MENU: &str = "+15125550910";
const VERSIONED: &str = "+15125550911";
const KEYPAD: &str = "+15125550912";
/// What the recognizer hears in each tone.
const SALES_HZ: u32 = 700;
const SALES: &str = "Sales, please.";
const LATER_HZ: u32 = 1100;
const LATER: &str = "I would like a quote for ten phones";
/// A tone the recognizer hears as no words.
const NOISE_HZ: u32 = 1500;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_menu_hears_words_digits_and_silence_and_a_call_is_transcribed() {
    let Some(db) = TestDatabase::create().await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let seed = Seed::new(db.url(), REALM).await;
    let Node {
        service,
        sip_port,
        scripted,
        events,
        _dir,
    } = Node::start(&db, &[(SALES_HZ, SALES), (LATER_HZ, LATER)]).await;

    let alice = seed.user("alice", "alice-pw", "1001").await;
    let storage = seed.storage();

    let flow: fcp_flow::FlowDefinition = serde_json::from_value(serde_json::json!({
        "id": "menu",
        "name": "Menu",
        "version": 1,
        "initial_node": "answer",
        "enabled": true,
        "tenant_id": seed.tenant_id.to_string(),
        "nodes": {
            "answer": {"type": "answer", "next": "menu"},
            "menu": {
                "type": "gather",
                "input": "dtmf_speech",
                "num_digits": 1,
                "timeout": 3,
                "hints": ["sales", "support"],
                "language": "en-US",
                "transitions": {"2": "support"},
                "speech_transitions": {"sales": "sales", "support": "support"},
                "on_timeout": "nothing"
            },
            "sales": {"type": "transcribe", "next": "listen"},
            "listen": {"type": "pause", "duration": 4, "next": "stop"},
            "stop": {"type": "transcribe", "action": "stop", "next": "bye"},
            "support": {"type": "log", "message": "support", "next": "bye"},
            "nothing": {"type": "log", "message": "nothing said", "next": "bye"},
            "bye": {"type": "hangup"}
        }
    }))
    .unwrap();
    fcp_flow::FlowStore::insert_flow(storage, flow)
        .await
        .unwrap();
    let number = fcp_core::PhoneNumber::new(MENU.to_string(), seed.tenant_id).unwrap();
    let mut binding =
        fcp_core::LineBinding::new(number.id, None, fcp_core::BindingType::Ivr, seed.tenant_id);
    binding.flow_id = Some("menu".to_string());
    storage.put_phone_number(number).await.unwrap();
    storage.put_line_binding(binding).await.unwrap();

    let phone = Phone::new(
        &alice.username,
        &alice.password,
        REALM,
        SocketAddr::from(([127, 0, 0, 1], sip_port)),
    )
    .await
    .unwrap();
    assert_eq!(phone.register().await.unwrap(), 200);
    let traces =
        fcp_storage_postgres::flow_trace::PostgresFlowTraceStore::new(storage.pool().clone());

    // Said: "sales", then something the transcript carries.
    let call = phone.call(MENU).await.unwrap();
    call.answered(SOON).await.expect("the flow answers");
    tokio::time::sleep(Duration::from_millis(500)).await;
    call.media().play_tone(SALES_HZ as f32, 1200);
    tokio::time::sleep(Duration::from_millis(2500)).await;
    call.media().play_tone(LATER_HZ as f32, 1500);
    if !call.ended(Duration::from_secs(10)).await {
        call.hangup().await.unwrap();
    }
    let path = nodes_run(&traces).await;
    if !(path.contains(&"sales".to_string()) && path.contains(&"stop".to_string())) {
        panic!(
            "the spoken choice: {path:?}\nrecognizer: {:?}\n{}",
            scripted.requests(),
            service.kill().await
        );
    }
    let transcript = events
        .wait_for(|e| e["type"] == "transcript" && e["text"] == LATER)
        .await
        .unwrap_or_else(|| panic!("no call.transcript: {:?}", events.all()));
    assert_eq!(transcript["leg"], "caller");
    assert_eq!(transcript["is_final"], true);
    assert_eq!(
        transcript["tenant_id"],
        seed.tenant_id.as_uuid().to_string()
    );
    // The menu's own words were not sent as the transcript.
    assert!(!events.all().iter().any(|e| e["text"] == SALES));
    let asked = scripted.requests();
    let gather = asked.iter().find(|r| r.answered.iter().any(|a| a == SALES));
    let gather = gather.unwrap_or_else(|| panic!("the menu was not recognised: {asked:?}"));
    assert_eq!(gather.kind, "stream");
    // The call's tenant named (docs/TENANT_RECOGNITION.md R1); the menu
    // names its own language, which wins over the tenant's.
    assert_eq!(gather.query["tenant_id"], seed.tenant_id.to_string());
    assert_eq!(gather.query["language"], "en-US");
    assert!(
        gather.query["hints"].contains("sales"),
        "{:?}",
        gather.query
    );

    // Pressed: 2, by SIP INFO.
    let call = phone.call(MENU).await.unwrap();
    call.answered(SOON).await.expect("the flow answers");
    tokio::time::sleep(Duration::from_millis(500)).await;
    let code = call.send_info_dtmf('2').await.unwrap();
    if code != 200 {
        panic!("INFO answered {code}\n{}", service.kill().await);
    }
    assert!(call.ended(SOON).await, "the support branch hangs up");
    let path = nodes_run(&traces).await;
    assert!(
        path.contains(&"support".to_string()),
        "the INFO digit: {path:?}"
    );

    // Silence: the timeout.
    let call = phone.call(MENU).await.unwrap();
    call.answered(SOON).await.expect("the flow answers");
    assert!(
        call.ended(Duration::from_secs(8)).await,
        "the menu times out"
    );
    let path = nodes_run(&traces).await;
    assert!(path.contains(&"nothing".to_string()), "silence: {path:?}");

    // A noise heard as speech and recognised as no words: the caller is
    // not waited for as if still speaking.
    let call = phone.call(MENU).await.unwrap();
    call.answered(SOON).await.expect("the flow answers");
    tokio::time::sleep(Duration::from_millis(500)).await;
    call.media().play_tone(NOISE_HZ as f32, 400);
    assert!(
        call.ended(Duration::from_secs(9)).await,
        "the menu times out after a noise"
    );
    let path = nodes_run(&traces).await;
    assert!(path.contains(&"nothing".to_string()), "a noise: {path:?}");

    service.stop().await;
    db.drop_db().await;
}

/// The key that finishes a Gather is not part of what was entered: kept,
/// the D8 keypad test read the pound back, and a transition on the digits
/// alone was never taken (`docs/DEEP_TESTING.md` D8).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_finish_key_is_not_part_of_the_digits() {
    let Some(db) = TestDatabase::create().await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let seed = Seed::new(db.url(), REALM).await;
    let Node {
        service,
        sip_port,
        _dir,
        ..
    } = Node::start(&db, &[]).await;
    let alice = seed.user("alice", "alice-pw", "1001").await;
    let storage = seed.storage();
    let flow: fcp_flow::FlowDefinition = serde_json::from_value(serde_json::json!({
        "id": "keypad",
        "name": "Keypad",
        "version": 1,
        "initial_node": "answer",
        "enabled": true,
        "tenant_id": seed.tenant_id.to_string(),
        "nodes": {
            "answer": {"type": "answer", "next": "keys"},
            "keys": {
                "type": "gather",
                "num_digits": 0,
                "finish_on_key": "#",
                "timeout": 5,
                "variable": "keys",
                "transitions": {"42": "right"},
                "default": "wrong",
                "on_timeout": "wrong"
            },
            "right": {"type": "log", "message": "42", "next": "bye"},
            "wrong": {"type": "log", "message": "not 42", "next": "bye"},
            "bye": {"type": "hangup"}
        }
    }))
    .unwrap();
    fcp_flow::FlowStore::insert_flow(storage, flow)
        .await
        .unwrap();
    let number = fcp_core::PhoneNumber::new(KEYPAD.to_string(), seed.tenant_id).unwrap();
    let mut binding =
        fcp_core::LineBinding::new(number.id, None, fcp_core::BindingType::Ivr, seed.tenant_id);
    binding.flow_id = Some("keypad".to_string());
    storage.put_phone_number(number).await.unwrap();
    storage.put_line_binding(binding).await.unwrap();

    let phone = Phone::new(
        &alice.username,
        &alice.password,
        REALM,
        SocketAddr::from(([127, 0, 0, 1], sip_port)),
    )
    .await
    .unwrap();
    assert_eq!(phone.register().await.unwrap(), 200);
    let traces =
        fcp_storage_postgres::flow_trace::PostgresFlowTraceStore::new(storage.pool().clone());

    let call = phone.call(KEYPAD).await.unwrap();
    call.answered(SOON).await.expect("the flow answers");
    tokio::time::sleep(Duration::from_millis(500)).await;
    for digit in ['4', '2', '#'] {
        assert_eq!(call.send_info_dtmf(digit).await.unwrap(), 200);
    }
    assert!(call.ended(SOON).await, "the flow hangs up");
    let (path, _) = newest_call(&traces, "keypad").await;
    if !path.contains(&"right".to_string()) {
        panic!("4, 2, #: {path:?}\n{}", service.kill().await);
    }
    service.stop().await;
    db.drop_db().await;
}

/// A call runs the version of its flow it started on
/// (`docs/DEVELOPER_SURFACE.md` phase 7b): the flow edited while the call
/// is in it does not change what the call does, and the next call runs
/// the edit. Both versions are kept.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_call_runs_the_version_it_started_on() {
    let Some(db) = TestDatabase::create().await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let seed = Seed::new(db.url(), REALM).await;
    let node = Node::start(&db, &[]).await;
    let alice = seed.user("alice", "alice-pw", "1001").await;
    let storage = seed.storage();

    let version = |said: &str| -> fcp_flow::FlowDefinition {
        serde_json::from_value(serde_json::json!({
            "id": "versioned",
            "name": "Versioned",
            "version": 1,
            "initial_node": "answer",
            "enabled": true,
            "tenant_id": seed.tenant_id.to_string(),
            "nodes": {
                "answer": {"type": "answer", "next": "wait"},
                "wait": {"type": "pause", "duration": 2, "next": said},
                said: {"type": "log", "message": said, "next": "bye"},
                "bye": {"type": "hangup"}
            }
        }))
        .unwrap()
    };
    fcp_flow::FlowStore::insert_flow(storage, version("one"))
        .await
        .unwrap();
    let number = fcp_core::PhoneNumber::new(VERSIONED.to_string(), seed.tenant_id).unwrap();
    let mut binding =
        fcp_core::LineBinding::new(number.id, None, fcp_core::BindingType::Ivr, seed.tenant_id);
    binding.flow_id = Some("versioned".to_string());
    storage.put_phone_number(number).await.unwrap();
    storage.put_line_binding(binding).await.unwrap();

    let phone = Phone::new(
        &alice.username,
        &alice.password,
        REALM,
        SocketAddr::from(([127, 0, 0, 1], node.sip_port)),
    )
    .await
    .unwrap();
    assert_eq!(phone.register().await.unwrap(), 200);
    let traces =
        fcp_storage_postgres::flow_trace::PostgresFlowTraceStore::new(storage.pool().clone());

    // The flow is edited while the call waits in it.
    let call = phone.call(VERSIONED).await.unwrap();
    call.answered(SOON).await.expect("the flow answers");
    fcp_flow::FlowStore::update_flow(storage, version("two"))
        .await
        .unwrap();
    assert!(
        call.ended(Duration::from_secs(8)).await,
        "the flow hangs up"
    );
    let (nodes, ran) = newest_call(&traces, "versioned").await;
    if !nodes.contains(&"one".to_string()) {
        panic!("{nodes:?}\n{}", node.service.kill().await);
    }
    assert_eq!(ran, Some(1), "{nodes:?}");

    // The next call runs the edit.
    let call = phone.call(VERSIONED).await.unwrap();
    call.answered(SOON).await.expect("the flow answers");
    assert!(
        call.ended(Duration::from_secs(8)).await,
        "the flow hangs up"
    );
    let (nodes, ran) = newest_call(&traces, "versioned").await;
    assert!(nodes.contains(&"two".to_string()), "{nodes:?}");
    assert_eq!(ran, Some(2));

    // Both versions are kept, each as it was.
    let kept = fcp_flow::FlowStore::list_flow_versions(storage, "versioned")
        .await
        .unwrap();
    assert_eq!(
        kept.iter().map(|f| f.version).collect::<Vec<_>>(),
        vec![2, 1]
    );
    let first = fcp_flow::FlowStore::get_flow_version(storage, "versioned", 1)
        .await
        .unwrap()
        .unwrap();
    assert!(first.nodes.contains_key("one") && !first.nodes.contains_key("two"));

    node.service.stop().await;
    db.drop_db().await;
}

/// The nodes the flow ran for the newest call (the trace is keyed by the
/// call manager's own id, not the SIP Call-ID). A Hangup node ends the
/// flow rather than being recorded as run.
async fn nodes_run(
    traces: &fcp_storage_postgres::flow_trace::PostgresFlowTraceStore,
) -> Vec<String> {
    newest_call(traces, "menu").await.0
}

/// The newest call's run of `flow_id`: the nodes it ran, and the flow
/// version it ran them from.
async fn newest_call(
    traces: &fcp_storage_postgres::flow_trace::PostgresFlowTraceStore,
    flow_id: &str,
) -> (Vec<String>, Option<u32>) {
    // Traces are written off the call path; give them a moment.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let (entries, _) = traces
        .query_flow_traces(&FlowTraceQuery {
            flow_id: Some(flow_id.to_string()),
            event_type: Some(FlowTraceEventType::NodeExecuted),
            limit: Some(1000),
            ..Default::default()
        })
        .await
        .unwrap();
    type Run = (std::time::SystemTime, Vec<String>, Option<u32>);
    let mut calls: BTreeMap<String, Run> = BTreeMap::new();
    for entry in entries {
        let call = calls.entry(entry.call_id.clone()).or_insert((
            entry.timestamp,
            Vec::new(),
            entry.flow_version,
        ));
        call.0 = call.0.min(entry.timestamp);
        call.1.extend(entry.node_id);
    }
    calls
        .into_values()
        .max_by_key(|(started, _, _)| *started)
        .map(|(_, nodes, version)| (nodes, version))
        .unwrap_or_default()
}

/// A call manager with speech recognition and its events forwarded.
struct Node {
    service: Service,
    sip_port: u16,
    scripted: ScriptedStt,
    events: EventSink,
    _dir: tempfile::TempDir,
}

impl Node {
    async fn start(db: &TestDatabase, script: &[(u32, &str)]) -> Self {
        let scripted = ScriptedStt::start("stt-token", script).await;
        let events = EventSink::start().await;
        let sip_port = free_port();
        let http = free_port();
        let rtp_start = allocate_rtp_range(40);
        let dir = tempfile::tempdir().unwrap();
        let config = format!(
            r#"
    [service]
    node_id = "e2e-listen"

    [sip]
    udp_bind = "127.0.0.1:{sip_port}"
    tcp_bind = "127.0.0.1:{sip_port}"
    local_ip = "127.0.0.1"

    [registrar]
    auth_enabled = true
    realm = "{REALM}"

    [storage]
    database_url = "{db}"

    [cdr]
    spool_dir = "{dir}/cdr-spool"

    [http]
    bind_address = "127.0.0.1:{http}"

    [media]
    rtp_port_start = {rtp_start}
    rtp_port_end = {rtp_end}
    prompts_dir = "{dir}/prompts"
    tts_cache_dir = "{dir}/tts-cache"
    recording_dir = "{dir}/recordings"

    [stt.provider]
    type = "remote"
    url = "{stt}"
    auth_token = "stt-token"

    [events]
    server_url = "{events}"

    [logging]
    level = "info"
    format = "compact"
    "#,
            db = db.url(),
            rtp_end = rtp_start + 40,
            dir = dir.path().display(),
            stt = scripted.url(),
            events = events.url,
        );
        let service =
            Service::start("call-manager", BIN, &config, &loopback(http, "/health")).await;
        Self {
            service,
            sip_port,
            scripted,
            events,
            _dir: dir,
        }
    }
}

/// A stand-in events server: every event the node forwards.
struct EventSink {
    url: String,
    events: Arc<Mutex<Vec<serde_json::Value>>>,
}

impl EventSink {
    async fn start() -> Self {
        let events = Arc::new(Mutex::new(Vec::new()));
        let router = Router::new()
            .route(
                "/ingest",
                post(
                    |State(events): State<Arc<Mutex<Vec<serde_json::Value>>>>,
                     Json(event): Json<serde_json::Value>| async move {
                        events.lock().unwrap().push(event);
                        axum::http::StatusCode::ACCEPTED
                    },
                ),
            )
            .with_state(Arc::clone(&events));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/ingest", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        Self { url, events }
    }

    /// Every event, its payload flattened to the variant's fields.
    fn all(&self) -> Vec<serde_json::Value> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .map(|e| e.get("data").cloned().unwrap_or_else(|| e.clone()))
            .collect()
    }

    async fn wait_for(
        &self,
        matches: impl Fn(&serde_json::Value) -> bool,
    ) -> Option<serde_json::Value> {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if let Some(found) = self.all().into_iter().find(|e| matches(e)) {
                return Some(found);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        None
    }
}
