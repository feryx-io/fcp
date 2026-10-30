//! A message left with the real voicemail server (`docs/DEEP_TESTING.md`
//! D8). Every other test reaches voicemail through a stub phone, so two
//! bugs a real deployment met went unseen: the server never made its
//! directories, and refused to record into a fresh volume; and a caller's
//! hang-up is what ends the message. Here a caller dials a mailbox's
//! number, speaks, hangs up, and the message is stored with its audio.
//! Skipped without `FCP_TEST_DATABASE_URL`.

use fcp_core::storage::{CredentialStore, LineBindingStore, PhoneNumberStore, VoicemailStore};
use fcp_core::ConfigStore;
use fcp_testkit::smoke::{self, free_port, loopback};
use fcp_testkit::{Phone, Seed, Service, TestDatabase};
use std::net::SocketAddr;
use std::time::Duration;

const BIN: &str = env!("CARGO_BIN_EXE_fcp-voicemail");
const REALM: &str = "vm.local";
const NUMBER: &str = "+15125550401";
const SOON: Duration = Duration::from_secs(10);

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_caller_leaves_a_message_and_it_is_kept() {
    let Some(db) = TestDatabase::create().await else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping");
        return;
    };
    let seed = Seed::new(db.url(), REALM).await;
    seed.user("alice", "Alice-pw-2026!", "2001").await;
    let storage = seed.storage();
    let alice = storage
        .get_user_in_tenant("alice", &seed.tenant_id)
        .await
        .unwrap()
        .unwrap();
    let number = fcp_core::PhoneNumber::new(NUMBER.into(), seed.tenant_id).unwrap();
    let binding = fcp_core::LineBinding::new(
        number.id,
        Some(alice.id),
        fcp_core::BindingType::User,
        seed.tenant_id,
    );
    let mailbox = fcp_core::VoicemailMailbox::new(
        seed.tenant_id,
        alice.id,
        binding.id,
        "2001".into(),
        "1234".into(),
    );
    let mailbox_id = mailbox.id.to_string();
    storage.put_phone_number(number).await.unwrap();
    storage.put_line_binding(binding).await.unwrap();
    storage.put_mailbox(mailbox).await.unwrap();

    // Directories nothing has made: a fresh volume.
    let dir = smoke::temp_dir();
    let recordings = dir.path().join("voicemail/recordings");
    let sip = free_port();
    let http = free_port();
    let config = format!(
        r#"[service]
node_id = "vm-messages"

[sip]
udp_bind = "127.0.0.1:{sip}"
tcp_bind = "127.0.0.1:{sip}"
local_ip = "127.0.0.1"
local_uri = "sip:voicemail@127.0.0.1:{sip}"

[storage]
database_url = "{db}"

[http]
bind_address = "127.0.0.1:{http}"

[voicemail]
recordings_dir = "{recordings}"
greetings_dir = "{dir}/voicemail/greetings"

[logging]
level = "info"
format = "compact"
"#,
        db = db.url(),
        dir = smoke::toml_path(dir.path()),
        recordings = smoke::toml_path(&recordings),
    );
    let server = Service::start("voicemail", BIN, &config, &loopback(http, "/health")).await;
    assert!(recordings.is_dir(), "the server made its directory");

    let caller = Phone::new(
        "caller",
        "x",
        REALM,
        SocketAddr::from(([127, 0, 0, 1], sip)),
    )
    .await
    .expect("caller binds");
    let call = caller
        .call(&format!("sip:{NUMBER}@127.0.0.1:{sip}"))
        .await
        .expect("INVITE sent");
    call.answered(SOON).await.expect("the mailbox answers");
    // The beep, then the caller speaks for two seconds and hangs up.
    tokio::time::sleep(Duration::from_millis(800)).await;
    call.media().play_tone(440.0, 2000);
    tokio::time::sleep(Duration::from_millis(2200)).await;
    assert_eq!(call.hangup().await.expect("BYE"), 200);

    let deadline = tokio::time::Instant::now() + SOON;
    let message = loop {
        let messages = storage.list_messages(&mailbox_id).await.unwrap();
        if let Some(message) = messages.into_iter().next() {
            break message;
        }
        if tokio::time::Instant::now() > deadline {
            let logs = server.kill().await;
            db.drop_db().await;
            panic!("no message was kept\n{logs}");
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    let file = &message.recording_path;
    assert!(file.is_file(), "the message's audio is written: {file:?}");
    assert!(
        message.duration < 30,
        "the hang-up ended the message, not its limit: {}s",
        message.duration
    );
    server.stop().await;
    db.drop_db().await;
}

/// D8 findings 37–38: a call to Bob at 1002 that he declined or let ring
/// was forwarded as `sip:1002@voicemail` and failed — his mailbox has no
/// line, its extension is his username, and only the extensions dialing a
/// number were routed. A mailbox is reached by its id (what the call
/// manager forwards to now) and by the extension dialing its owner.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_mailbox_with_no_line_answers_its_id_and_its_owners_extension() {
    let Some(db) = TestDatabase::create().await else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping");
        return;
    };
    let seed = Seed::new(db.url(), REALM).await;
    seed.user("bob", "Bob-pw-2026!", "1002").await;
    let storage = seed.storage();
    let bob = storage
        .get_user_in_tenant("bob", &seed.tenant_id)
        .await
        .unwrap()
        .unwrap();
    let mailbox =
        fcp_core::VoicemailMailbox::new(seed.tenant_id, bob.id, None, "bob".into(), "1234".into());
    let mailbox_id = mailbox.id.to_string();
    storage.put_mailbox(mailbox).await.unwrap();

    let dir = smoke::temp_dir();
    let (sip, http) = (free_port(), free_port());
    let config = config(db.url(), dir.path(), sip, http, "");
    let server = Service::start("voicemail", BIN, &config, &loopback(http, "/health")).await;
    let caller = Phone::new(
        "caller",
        "x",
        REALM,
        SocketAddr::from(([127, 0, 0, 1], sip)),
    )
    .await
    .expect("caller binds");

    for user in ["1002", mailbox_id.as_str()] {
        for reason in ["no-answer", "busy"] {
            let call = caller
                .call(&format!("sip:{user}@127.0.0.1:{sip};reason={reason}"))
                .await
                .expect("INVITE sent");
            if call.answered(SOON).await.is_err() {
                let logs = server.kill().await;
                db.drop_db().await;
                panic!("the mailbox did not answer as {user} ({reason})\n{logs}");
            }
            call.hangup().await.expect("BYE");
        }
    }
    server.stop().await;
    db.drop_db().await;
}

/// A voicemail server's configuration: `extra` is added to `[voicemail]`.
fn config(db: &str, dir: &std::path::Path, sip: u16, http: u16, extra: &str) -> String {
    format!(
        r#"[service]
node_id = "vm-config"

[sip]
udp_bind = "127.0.0.1:{sip}"
tcp_bind = "127.0.0.1:{sip}"
local_ip = "127.0.0.1"
local_uri = "sip:voicemail@127.0.0.1:{sip}"

[storage]
database_url = "{db}"

[http]
bind_address = "127.0.0.1:{http}"

[voicemail]
recordings_dir = "{dir}/recordings"
greetings_dir = "{dir}/greetings"
{extra}

[logging]
level = "info"
format = "compact"
"#,
        dir = smoke::toml_path(dir),
    )
}

/// The `[voicemail]` section's stored value, as the server left it.
async fn stored(storage: &fcp_storage_postgres::PostgresStorage) -> serde_json::Value {
    storage
        .get_config("voicemail", "voicemail", "_system")
        .await
        .unwrap()
        .expect("the section is stored")
        .value
}

/// D8 found a greeting the TOML named never played: the section was stored
/// on the first boot, before the file had the key, and every boot after
/// read the stored row alone. A key the file sets later is read; an edit
/// made through the API stands while the file leaves that key alone.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_key_added_to_the_toml_later_is_read_and_an_api_edit_stands() {
    let Some(db) = TestDatabase::create().await else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping");
        return;
    };
    let seed = Seed::new(db.url(), REALM).await;
    let storage = seed.storage();
    let dir = smoke::temp_dir();
    let boot = |extra: &'static str| {
        let (sip, http) = (free_port(), free_port());
        let config = config(db.url(), dir.path(), sip, http, extra);
        async move { Service::start("voicemail", BIN, &config, &loopback(http, "/health")).await }
    };

    // The first boot, with no greeting in the file.
    boot("").await.stop().await;
    assert!(stored(storage).await["default_greeting_path"].is_null());

    // The file names a greeting: the next boot reads it.
    let greeting = r#"default_greeting_path = "/var/fcp/audio/default-greeting.wav""#;
    boot(greeting).await.stop().await;
    let value = stored(storage).await;
    assert_eq!(
        value["default_greeting_path"], "/var/fcp/audio/default-greeting.wav",
        "the file's new key is read: {value}"
    );

    // An edit through the API stands across a boot with the same file.
    let mut edited = value.clone();
    edited["max_message_duration"] = serde_json::json!(60);
    storage
        .set_config("voicemail", "voicemail", "_system", edited, Some("api"))
        .await
        .unwrap();
    boot(greeting).await.stop().await;
    let value = stored(storage).await;
    assert_eq!(
        value["max_message_duration"], 60,
        "the edit stands: {value}"
    );
    assert_eq!(
        value["default_greeting_path"],
        "/var/fcp/audio/default-greeting.wav"
    );
    db.drop_db().await;
}
