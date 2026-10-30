//! Subscribed message waiting (RFC 3842, `docs/SOFTPHONE.md` phase A3): a
//! phone SUBSCRIBEs to `message-summary` and is told its mailbox's counts
//! at once, told again when a message arrives (`voicemail.mwi_updated` on
//! the event bus), and told the subscription is over when it ends it. A
//! phone with no mailbox is answered 404. Skipped without
//! `FCP_TEST_DATABASE_URL` and `FCP_TEST_REDIS_URL`.

use fcp_core::storage::{CredentialStore, LineBindingStore, PhoneNumberStore, VoicemailStore};
use fcp_events::{EventPublisher, PlatformEvent, VoicemailEvent};
use fcp_testkit::cluster::private_redis_url;
use fcp_testkit::media::allocate_port_pair;
use fcp_testkit::smoke::loopback;
use fcp_testkit::{free_port, Phone, PhoneEvent, Seed, Service, TestDatabase};
use std::net::SocketAddr;
use std::time::{Duration, SystemTime};

const BIN: &str = env!("CARGO_BIN_EXE_fcp-call-manager");
const REALM: &str = "e2e.local";
const SOON: Duration = Duration::from_secs(5);

/// A call manager on Redis's event bus, as the compose stack runs one.
async fn node(db: &str, redis: &str, dir: &std::path::Path) -> (Service, SocketAddr) {
    std::fs::create_dir_all(dir.join("voicemail")).unwrap();
    let (sip_port, http) = (free_port(), free_port());
    let rtp_start = allocate_port_pair().rtp_port;
    let config = format!(
        r#"
[service]
node_id = "e2e-mwi"

[sip]
udp_bind = "127.0.0.1:{sip_port}"
tcp_bind = "127.0.0.1:{sip_port}"
local_ip = "127.0.0.1"

[registrar]
auth_enabled = true
realm = "{REALM}"

[storage]
database_url = "{db}"
redis_url = "{redis}"

[events]
use_redis = true

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
voicemail_recordings_dir = "{dir}/voicemail"

[logging]
level = "info"
format = "compact"
"#,
        rtp_end = rtp_start + 20,
        dir = dir.display(),
    );
    let service = Service::start("call-manager", BIN, &config, &loopback(http, "/health")).await;
    let sip = SocketAddr::from(([127, 0, 0, 1], sip_port));
    (service, sip)
}

/// The next message-summary NOTIFY's body.
async fn summary(phone: &Phone) -> Option<String> {
    let deadline = tokio::time::Instant::now() + SOON;
    while tokio::time::Instant::now() < deadline {
        if let Some(PhoneEvent::Notify { event, body, .. }) =
            phone.next_event(Duration::from_millis(250)).await
        {
            if event.starts_with("message-summary") {
                return Some(body);
            }
        }
    }
    None
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_subscribed_phone_is_told_its_messages() {
    let (Some(db), Some(redis)) = (TestDatabase::create().await, private_redis_url()) else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL or FCP_TEST_REDIS_URL is not set");
        return;
    };
    let seed = Seed::new(db.url(), REALM).await;
    let dir = tempfile::tempdir().unwrap();
    let (service, sip) = node(db.url(), &redis, dir.path()).await;

    // Alice has a mailbox with two new messages; Bob has none.
    let alice = seed.user("alice", "Alice-pw-2026!", "2001").await;
    let bob = seed.user("bob", "Bob-pw-2026!", "2002").await;
    let storage = seed.storage();
    let row = storage
        .get_user_in_tenant("alice", &seed.tenant_id)
        .await
        .unwrap()
        .unwrap();
    let number = fcp_core::PhoneNumber::new("+15125550301".into(), seed.tenant_id).unwrap();
    let binding = fcp_core::LineBinding::new(
        number.id,
        Some(row.id),
        fcp_core::BindingType::User,
        seed.tenant_id,
    );
    let mailbox = fcp_core::VoicemailMailbox::new(
        seed.tenant_id,
        row.id,
        binding.id,
        "2001".into(),
        "1234".into(),
    );
    let mailbox_id = mailbox.id.to_string();
    storage.put_phone_number(number).await.unwrap();
    storage.put_line_binding(binding).await.unwrap();
    storage.put_mailbox(mailbox).await.unwrap();
    let message = |n: u8| {
        fcp_core::VoicemailMessage::new(
            mailbox_id.clone(),
            format!("sip:+1512555090{n}@pstn"),
            10,
            dir.path().join(format!("m{n}.wav")),
        )
    };
    storage.put_message(message(1)).await.unwrap();
    storage.put_message(message(2)).await.unwrap();

    // Subscribed: the counts at once.
    let phone = Phone::new(&alice.username, &alice.password, REALM, sip)
        .await
        .unwrap();
    let mut sub = phone
        .subscribe(&alice.username, "message-summary", 600, None)
        .await
        .unwrap();
    assert_eq!(sub.code, 200, "the subscription is accepted");
    let body = summary(&phone).await.expect("the counts at once");
    assert!(body.contains("Messages-Waiting: yes"), "{body}");
    assert!(body.contains("Voice-Message: 2/0"), "{body}");

    // A third message, announced on the bus as the voicemail flow does.
    let third = message(3);
    storage.put_message(third.clone()).await.unwrap();
    let bus = fcp_storage_redis::event_bus::RedisEventBus::new(
        fcp_storage_redis::RedisStorage::new(fcp_storage_redis::RedisConfig {
            url: redis.clone(),
            ..Default::default()
        })
        .await
        .unwrap(),
        "e2e-voicemail".into(),
    );
    bus.publish(PlatformEvent::Voicemail(VoicemailEvent::MessageRecorded {
        tenant_id: Some(seed.tenant_id),
        call_id: fcp_core::CallId::new(),
        mailbox_id: mailbox_id.clone(),
        message_id: third.id.clone(),
        caller: third.caller.clone(),
        duration_secs: 10,
        timestamp: SystemTime::now(),
    }))
    .await
    .unwrap();
    let body = summary(&phone).await.expect("told of the new message");
    assert!(body.contains("Voice-Message: 3/0"), "{body}");

    // Heard through the API, which announces the counts.
    let mut heard = third.clone();
    heard.status = fcp_core::MessageStatus::Heard;
    storage.put_message(heard).await.unwrap();
    bus.publish(PlatformEvent::Voicemail(VoicemailEvent::MwiUpdated {
        tenant_id: Some(seed.tenant_id),
        mailbox_id: mailbox_id.clone(),
        new_messages: 2,
        old_messages: 1,
        timestamp: SystemTime::now(),
    }))
    .await
    .unwrap();
    let body = summary(&phone).await.expect("told it was heard");
    assert!(body.contains("Voice-Message: 2/1"), "{body}");

    // Deleted.
    storage.delete_message(&third.id).await.unwrap();
    bus.publish(PlatformEvent::Voicemail(VoicemailEvent::MessageDeleted {
        tenant_id: Some(seed.tenant_id),
        mailbox_id: mailbox_id.clone(),
        message_id: third.id.clone(),
        timestamp: SystemTime::now(),
    }))
    .await
    .unwrap();
    let body = summary(&phone).await.expect("told it was deleted");
    assert!(body.contains("Voice-Message: 2/0"), "{body}");

    // A refresh of a subscription the node does not hold — one a restart
    // took — is 481, so the phone subscribes afresh. Challenged instead, a
    // VVX resent its refresh on a dead nonce for ever (D8, finding 39).
    let mut forgotten = sub.clone();
    forgotten.call_id = format!("{}@127.0.0.1", uuid::Uuid::new_v4().simple());
    assert_eq!(
        phone.resubscribe(&mut forgotten, 600, None).await.unwrap(),
        481,
        "a refresh for no subscription here"
    );

    // Ended: one last NOTIFY.
    assert_eq!(phone.resubscribe(&mut sub, 0, None).await.unwrap(), 200);
    assert!(summary(&phone).await.is_some(), "the closing NOTIFY");

    // No mailbox, no subscription.
    let bobs = Phone::new(&bob.username, &bob.password, REALM, sip)
        .await
        .unwrap();
    let none = bobs
        .subscribe(&bob.username, "message-summary", 600, None)
        .await
        .unwrap();
    assert_eq!(none.code, 404);

    service.stop().await;
    db.drop_db().await;
}

/// Key `digits` into the call once the node is listening for them.
async fn key(call: &fcp_testkit::Call, digits: &str) {
    tokio::time::sleep(Duration::from_millis(800)).await;
    for digit in digits.chars() {
        call.media().send_dtmf(digit);
    }
}

/// `*98` from a phone reaches its user's own mailbox, whatever the
/// mailbox's extension (D8: alice's, made as `alice`, was looked for as
/// her extension and every PIN was refused), and a message deleted there
/// — a file the voicemail server wrote, read here from its directory —
/// puts the lamp out: leaving the menu announces the counts
/// (`voicemail.mwi_updated`), which the node tells its subscribers.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_voicemail_menu_opens_the_callers_mailbox_and_puts_the_lamp_out() {
    let (Some(db), Some(redis)) = (TestDatabase::create().await, private_redis_url()) else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL or FCP_TEST_REDIS_URL is not set");
        return;
    };
    let seed = Seed::new(db.url(), REALM).await;
    let dir = tempfile::tempdir().unwrap();
    let (service, sip) = node(db.url(), &redis, dir.path()).await;

    let alice = seed.user("alice", "Alice-pw-2026!", "1001").await;
    let storage = seed.storage();
    let row = storage
        .get_user_in_tenant("alice", &seed.tenant_id)
        .await
        .unwrap()
        .unwrap();
    let number = fcp_core::PhoneNumber::new("+15125550302".into(), seed.tenant_id).unwrap();
    let binding = fcp_core::LineBinding::new(
        number.id,
        Some(row.id),
        fcp_core::BindingType::User,
        seed.tenant_id,
    );
    let mailbox = fcp_core::VoicemailMailbox::new(
        seed.tenant_id,
        row.id,
        binding.id,
        "alice".into(),
        "4826".into(),
    );
    let mailbox_id = mailbox.id.to_string();
    storage.put_phone_number(number).await.unwrap();
    storage.put_line_binding(binding).await.unwrap();
    storage.put_mailbox(mailbox).await.unwrap();
    // Recorded by the voicemail server, in its directory.
    let recording = dir.path().join("voicemail/m.wav");
    std::fs::write(&recording, b"").unwrap();
    let message = fcp_core::VoicemailMessage::new(
        mailbox_id.clone(),
        "sip:+15125550901@pstn".into(),
        10,
        recording,
    );
    storage.put_message(message).await.unwrap();

    let phone = Phone::new(&alice.username, &alice.password, REALM, sip)
        .await
        .unwrap();
    assert_eq!(phone.register().await.unwrap(), 200);
    let sub = phone
        .subscribe(&alice.username, "message-summary", 600, None)
        .await
        .unwrap();
    assert_eq!(sub.code, 200);
    let body = summary(&phone).await.expect("the counts at once");
    assert!(body.contains("Voice-Message: 1/0"), "{body}");

    // The PIN, new messages, delete, back to the menu, out.
    let call = phone.call("*98").await.unwrap();
    call.answered(SOON).await.expect("voicemail access answers");
    key(&call, "4826#").await;
    key(&call, "1").await;
    key(&call, "3").await;
    key(&call, "*").await;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    while !storage.list_messages(&mailbox_id).await.unwrap().is_empty() {
        if tokio::time::Instant::now() > deadline {
            let logs = service.kill().await;
            panic!("the message was not deleted: the mailbox was not opened\n{logs}");
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let mut out = None;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    while out.is_none() && tokio::time::Instant::now() < deadline {
        out = summary(&phone)
            .await
            .filter(|b| b.contains("Voice-Message: 0/0"));
    }
    if out.is_none() {
        let logs = service.kill().await;
        panic!("the lamp was not put out\n{logs}");
    }
    let _ = call.hangup().await;
    service.stop().await;
    db.drop_db().await;
}
