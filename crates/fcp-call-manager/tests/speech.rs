//! Speech recognition against the running binary
//! (`docs/DEVELOPER_SURFACE.md` phase 6): a caller leaves a message
//! through a flow's Voicemail node, and the message is transcribed by the
//! configured provider — the testkit's scripted recognizer behind the
//! `remote` provider, which "hears" a phrase for a tone — when its mailbox
//! asks, and not when it does not. Skipped without `FCP_TEST_DATABASE_URL`.

use fcp_core::storage::{LineBindingStore, PhoneNumberStore, VoicemailStore};
use fcp_testkit::media::allocate_port_pair;
use fcp_testkit::smoke::loopback;
use fcp_testkit::stt::ScriptedStt;
use fcp_testkit::{free_port, Phone, Seed, Service, TestDatabase};
use std::net::SocketAddr;
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_fcp-call-manager");
const REALM: &str = "e2e.local";
const SOON: Duration = Duration::from_secs(5);
/// The tone the caller plays, and what the recognizer hears in it.
const SPOKEN_HZ: u32 = 700;
const SPOKEN: &str = "please call me back about the invoice";

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_message_is_transcribed_when_its_mailbox_asks() {
    let Some(db) = TestDatabase::create().await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let seed = Seed::new(db.url(), REALM).await;
    let scripted = ScriptedStt::start("stt-token", &[(SPOKEN_HZ, SPOKEN)]).await;
    let sip_port = free_port();
    let http = free_port();
    let rtp_start = allocate_port_pair().rtp_port;
    let dir = tempfile::tempdir().unwrap();
    let config = format!(
        r#"
[service]
node_id = "e2e-speech"

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

[stt]
language = "en-GB"

[stt.provider]
type = "remote"
url = "{stt}"
auth_token = "stt-token"

[logging]
level = "info"
format = "compact"
"#,
        db = db.url(),
        rtp_end = rtp_start + 40,
        dir = dir.path().display(),
        stt = scripted.url(),
    );
    let service = Service::start("call-manager", BIN, &config, &loopback(http, "/health")).await;

    let alice = seed.user("alice", "alice-pw", "1001").await;
    let bob = seed.user("bob", "bob-pw", "1002").await;
    let storage = seed.storage();

    // Bob's mailbox, transcribed; a number whose flow takes a message.
    let bob_row =
        fcp_core::storage::CredentialStore::get_user_in_tenant(storage, "bob", &seed.tenant_id)
            .await
            .unwrap()
            .unwrap();
    let mailbox_number =
        fcp_core::PhoneNumber::new("+15125550900".to_string(), seed.tenant_id).unwrap();
    let mailbox_binding = fcp_core::LineBinding::new(
        mailbox_number.id,
        Some(bob_row.id),
        fcp_core::BindingType::User,
        seed.tenant_id,
    );
    let mut mailbox = fcp_core::VoicemailMailbox::new(
        seed.tenant_id,
        bob_row.id,
        mailbox_binding.id,
        bob.extension.clone(),
        "1234".to_string(),
    );
    mailbox.transcription_enabled = true;
    let mailbox_id = mailbox.id.clone();
    storage.put_phone_number(mailbox_number).await.unwrap();
    storage.put_line_binding(mailbox_binding).await.unwrap();
    storage.put_mailbox(mailbox.clone()).await.unwrap();

    let flow: fcp_flow::FlowDefinition = serde_json::from_value(serde_json::json!({
        "id": "leave-a-message",
        "name": "Leave a message",
        "version": 1,
        "initial_node": "answer",
        "enabled": true,
        "tenant_id": seed.tenant_id.to_string(),
        "nodes": {
            "answer": {"id": "answer", "name": "Answer", "type": "answer", "next": "record"},
            "record": {
                "id": "record", "name": "Record", "type": "voicemail",
                "mailbox": mailbox_id, "max_duration": 10,
                "next_on_success": "done", "next_on_error": "done"
            },
            "done": {"id": "done", "name": "Hang up", "type": "hangup"}
        }
    }))
    .unwrap();
    fcp_flow::FlowStore::insert_flow(storage, flow)
        .await
        .unwrap();
    let number = fcp_core::PhoneNumber::new("+15125550901".to_string(), seed.tenant_id).unwrap();
    let mut binding =
        fcp_core::LineBinding::new(number.id, None, fcp_core::BindingType::Ivr, seed.tenant_id);
    binding.flow_id = Some("leave-a-message".to_string());
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

    // Leave a message: the silence standing in for a greeting, the beep,
    // then three seconds "said", then hang up.
    let phone = &phone;
    let leave = |hz: f32| async move {
        let call = phone.call("+15125550901").await.unwrap();
        call.answered(SOON).await.expect("the flow answers");
        tokio::time::sleep(Duration::from_millis(1500)).await;
        call.media().play_tone(hz, 3000);
        tokio::time::sleep(Duration::from_millis(3300)).await;
        call.hangup().await.unwrap();
    };
    leave(SPOKEN_HZ as f32).await;

    let transcribed = wait_for(|| async {
        storage
            .list_messages(&mailbox_id)
            .await
            .unwrap()
            .into_iter()
            .find(|m| m.transcription.is_some())
    })
    .await;
    let Some(message) = transcribed else {
        panic!("no transcription\n{}", service.kill().await);
    };
    assert_eq!(message.transcription.as_deref(), Some(SPOKEN));
    assert!(std::path::Path::new(&message.recording_path).exists());
    let asked = scripted.requests();
    assert_eq!(asked.len(), 1, "one recording, recognised once");
    assert_eq!(asked[0].kind, "file");
    assert_eq!(asked[0].query["language"], "en-GB");
    assert!(asked[0].seconds >= 2.5, "{} s", asked[0].seconds);

    // A mailbox that does not ask is not sent to the recognizer.
    mailbox.transcription_enabled = false;
    storage.put_mailbox(mailbox).await.unwrap();
    leave(SPOKEN_HZ as f32).await;
    let two = wait_for(|| async {
        let messages = storage.list_messages(&mailbox_id).await.unwrap();
        (messages.len() == 2).then_some(messages)
    })
    .await
    .expect("the second message is saved");
    assert_eq!(two.iter().filter(|m| m.transcription.is_none()).count(), 1);
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(scripted.requests().len(), 1, "the recognizer was not asked");

    service.stop().await;
    db.drop_db().await;
}

async fn wait_for<T, F, Fut>(mut check: F) -> Option<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Option<T>>,
{
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if let Some(found) = check().await {
            return Some(found);
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    None
}
