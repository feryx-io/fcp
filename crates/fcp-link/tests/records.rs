//! Records up (`docs/HYBRID_CONTROL_PLANE.md` phase 5): what a site's stack
//! writes reaches the cloud through the link. A record written at the
//! site is stored in the cloud under the tenant, naming the site, rated
//! afresh and announced as `cdr.written`; a recording's and a voicemail's
//! rows land naming the site; the cloud's download of a recording the
//! site holds is answered with the site's file, through the link; a
//! recording deleted in the cloud is removed at the site; a message
//! deleted at the site is gone in the cloud; an event published at the
//! site reaches the cloud's events server named as the site's; and the
//! heartbeat says who is registered at the site. Gated on
//! `FCP_TEST_DATABASE_URL` and `FCP_TEST_REDIS_URL`; skipped with a notice
//! when unset.

mod common;

use common::{cloud_with_site_opts, eventually, stub_events_server, until, CloudOptions};
use fcp_core::recording::{CallRecording, CallRecordingSource, Recording, RecordingConsent};
use fcp_core::storage::{CdrKind, CdrQuery, CdrStore, RegistrationStore, VoicemailStore};
use fcp_core::{MessagePriority, MessageStatus, VoicemailMailbox, VoicemailMessage};
use fcp_testkit::smoke::{http_request, loopback, toml_path};
use fcp_testkit::TestDatabase;
use std::time::{Duration, SystemTime};

fn site_storage(
    url: &str,
) -> impl std::future::Future<Output = fcp_storage_postgres::PostgresStorage> {
    let url = url.to_string();
    async move {
        fcp_storage_postgres::PostgresStorage::new(fcp_storage_postgres::PostgresConfig {
            connection_string: url,
            read_replica_url: None,
            max_replica_lag: Duration::from_secs(5),
            max_connections: 4,
            connection_timeout: Duration::from_secs(30),
            encryption: None,
        })
        .await
        .expect("a database")
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn what_a_site_writes_reaches_the_cloud_and_its_files_are_reached_through_the_link() {
    let Some(site_redis) = fcp_testkit::cluster::private_redis_url() else {
        eprintln!("FCP_TEST_REDIS_URL not set; skipping the records test");
        return;
    };
    let Some(site_db) = TestDatabase::create().await else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping the records test");
        return;
    };
    let (events_port, posted) = stub_events_server().await;
    let files_dir = tempfile::tempdir().unwrap();
    let cloud = cloud_with_site_opts(CloudOptions {
        site_database_url: Some(site_db.url()),
        sip_host: None,
        events_server_url: Some(loopback(events_port, "/ingest")),
        site_redis_url: Some(site_redis.clone()),
        site_file_roots: vec![toml_path(files_dir.path())],
        ..Default::default()
    })
    .await
    .expect("the database was there a moment ago");
    let tenant_id: fcp_core::TenantId = cloud.tenant_id.parse().unwrap();
    let site_id: fcp_core::SiteId = cloud.site_id.parse().unwrap();
    let site = site_storage(site_db.url()).await;
    let cloud_store = site_storage(cloud.db.url()).await;
    let cloud_recordings =
        fcp_storage_postgres::recording::PostgresRecordingStore::new(cloud_store.pool().clone());
    let site_recordings =
        fcp_storage_postgres::recording::PostgresRecordingStore::new(site.pool().clone());

    // The site's stack wrote a record: rated by its own copy of the plans.
    let started = SystemTime::now() - Duration::from_secs(60);
    let mut record = fcp_cdr::RecordBuilder::new(CdrKind::Call, fcp_core::CallId::new(), tenant_id)
        .from("sip:alice@fcp.local", None)
        .to("sip:+15125550999@fcp.local", None)
        .started_at(started)
        .ended_at(Some(started + Duration::from_secs(30)))
        .build();
    record.node_id = Some("acme-hq-cm".to_string());
    record.cost = Some(fcp_core::billing::Decimal::new(999, 2));
    record.currency = Some("USD".to_string());
    let cdr_id = record.cdr_id;
    site.insert_cdrs(vec![record]).await.unwrap();

    // …a call recording, a conference recording, and a voicemail, with
    // their files under the link's roots.
    let call_file = files_dir.path().join("call.wav");
    std::fs::write(&call_file, b"RIFF-call").unwrap();
    let call_recording = CallRecording {
        id: fcp_core::RecordingId::new(),
        tenant_id,
        call_id: "call-1".to_string(),
        source: CallRecordingSource::Api,
        node_id: Some("acme-hq-cm".to_string()),
        site_id: None,
        path: call_file.to_string_lossy().to_string(),
        format: "wav".to_string(),
        consent: RecordingConsent::default(),
        started_at: SystemTime::now(),
        ended_at: Some(SystemTime::now()),
    };
    use fcp_core::recording::{CallRecordingStore, RecordingStore};
    site_recordings
        .create_call_recording(&call_recording)
        .await
        .unwrap();
    let conf_file = files_dir.path().join("room-100.wav");
    std::fs::write(&conf_file, b"RIFF-conference-audio").unwrap();
    let mut conference = Recording::starting(
        tenant_id,
        "100",
        "acme-hq-conf",
        conf_file.to_string_lossy().to_string(),
        fcp_core::recording::RecordingFormat::Wav,
        fcp_core::recording::RecordingTrigger::Api,
    );
    conference.ended_at = Some(SystemTime::now());
    conference.size_bytes = Some(21);
    let conference_id = conference.id;
    site_recordings.create_recording(&conference).await.unwrap();
    // A mailbox hangs off a line binding, made in the cloud — the system
    // of record — and synced down before the site's voicemail uses it.
    let alice_id = fcp_core::UserId::new();
    let number = fcp_core::PhoneNumber::new("+15125550111".to_string(), tenant_id).unwrap();
    let binding = fcp_core::LineBinding::new(
        number.id,
        Some(alice_id),
        fcp_core::BindingType::User,
        tenant_id,
    );
    let binding_id = binding.id;
    {
        use fcp_core::storage::{LineBindingStore, PhoneNumberStore};
        cloud_store.put_phone_number(number).await.unwrap();
        cloud_store.put_line_binding(binding).await.unwrap();
        eventually(
            "the line binding syncing to the site",
            Duration::from_secs(30),
            || async { site.get_line_binding(&binding_id).await.unwrap().is_some() },
        )
        .await;
    }
    let mailbox = VoicemailMailbox::new(
        tenant_id,
        alice_id,
        binding_id,
        "1001".to_string(),
        "1234".to_string(),
    );
    let mailbox_id = mailbox.id.clone();
    site.put_mailbox(mailbox).await.unwrap();
    let vm_file = files_dir.path().join("mailbox-1.wav");
    std::fs::write(&vm_file, b"RIFF-voicemail").unwrap();
    let message = VoicemailMessage {
        id: "vm-1".to_string(),
        mailbox_id: mailbox_id.clone(),
        caller: "sip:bob@fcp.local".to_string(),
        caller_name: Some("Bob".to_string()),
        status: MessageStatus::New,
        priority: MessagePriority::Normal,
        duration: 7,
        recording_path: vm_file.clone(),
        size_bytes: Some(14),
        site_id: None,
        transcription: None,
        created_at: SystemTime::now(),
        accessed_at: None,
    };
    site.put_message(message).await.unwrap();

    // The record lands in the cloud under the tenant, naming the site,
    // its rating set aside, and is announced naming the site.
    let cdr_shipped = Duration::from_secs(30);
    eventually("the record reaching the cloud", cdr_shipped, || async {
        cloud_store
            .query_cdrs(CdrQuery {
                tenant_id: Some(tenant_id),
                ..Default::default()
            })
            .await
            .unwrap()
            .iter()
            .any(|r| r.cdr_id == cdr_id && r.site_id == Some(site_id) && r.cost.is_none())
    })
    .await;
    eventually(
        "cdr.written naming the site",
        Duration::from_secs(10),
        || async {
            posted.lock().unwrap().iter().any(|p| {
                p.path == "/ingest"
                    && p.body["event_type"] == "cdr"
                    && p.body["type"] == "written"
                    && p.body["site_id"] == cloud.site_id
            })
        },
    )
    .await;

    // The rows land naming the site.
    eventually(
        "the recording rows reaching the cloud",
        cdr_shipped,
        || async {
            let call = cloud_recordings
                .get_call_recording(&call_recording.id)
                .await
                .unwrap();
            let conf = cloud_recordings
                .get_recording(&conference_id)
                .await
                .unwrap();
            let vm = cloud_store.get_message("vm-1").await.unwrap();
            call.is_some_and(|r| r.site_id == Some(site_id))
                && conf.is_some_and(|r| r.site_id == Some(site_id))
                && vm.is_some_and(|m| m.site_id == Some(site_id))
        },
    )
    .await;

    // The cloud's download of the conference recording is the site's
    // file, through the link.
    let (code, bytes) = {
        let client = reqwest::Client::new();
        let response = client
            .get(loopback(
                cloud.http,
                &format!("/api/v1/recordings/{conference_id}/download"),
            ))
            .bearer_auth(common::ADMIN_TOKEN)
            .header("x-fcp-tenant-id", &cloud.tenant_id)
            .send()
            .await
            .unwrap();
        (response.status().as_u16(), response.bytes().await.unwrap())
    };
    assert_eq!(code, 200, "{}", String::from_utf8_lossy(&bytes));
    assert_eq!(&bytes[..], b"RIFF-conference-audio");

    // Deleted in the cloud, the file goes at the site.
    let (code, answer) = cloud
        .admin_call(
            "DELETE",
            &format!("/api/v1/recordings/{conference_id}"),
            true,
            None,
        )
        .await;
    assert_eq!(code, 200, "{answer}");
    assert_eq!(answer["file_deleted"], true, "{answer}");
    eventually(
        "the site removing the file",
        Duration::from_secs(20),
        || async { !conf_file.exists() },
    )
    .await;

    // Deleted at the site, the message goes in the cloud.
    site.delete_message("vm-1").await.unwrap();
    eventually(
        "the cloud dropping the deleted message",
        cdr_shipped,
        || async { cloud_store.get_message("vm-1").await.unwrap().is_none() },
    )
    .await;

    // An event the site's stack published reaches the cloud's events
    // server named as the site's, under the node that published it.
    let site_bus = fcp_storage_redis::event_bus::RedisEventBus::new(
        fcp_storage_redis::RedisStorage::new(fcp_storage_redis::RedisConfig {
            url: site_redis.clone(),
            key_prefix: "fcp:".to_string(),
        })
        .await
        .unwrap(),
        "acme-hq-cm".to_string(),
    );
    site_bus
        .publish_many_from(
            "acme-hq-cm",
            vec![fcp_events::PlatformEvent::System(
                fcp_events::SystemEvent::ConfigChanged {
                    entity_type: "user".to_string(),
                    entity_id: "alice".to_string(),
                    tenant_id,
                    timestamp: SystemTime::now(),
                },
            )],
        )
        .await
        .unwrap();
    eventually(
        "the site's event reaching the cloud",
        Duration::from_secs(20),
        || async {
            posted.lock().unwrap().iter().any(|p| {
                p.path == "/ingest/batch"
                    && p.origin_site.as_deref() == Some(cloud.site_id.as_str())
                    && p.node_id.as_deref() == Some("acme-hq-cm")
                    && p.body.as_array().is_some_and(|events| {
                        events
                            .iter()
                            .any(|e| e["event_type"] == "system" && e["type"] == "config_changed")
                    })
            })
        },
    )
    .await;

    // Who is registered at the site is on the heartbeat.
    let registrations = fcp_storage_redis::registration::RedisRegistrationStore::new(
        fcp_storage_redis::RedisStorage::new(fcp_storage_redis::RedisConfig {
            url: site_redis.clone(),
            key_prefix: "fcp:".to_string(),
        })
        .await
        .unwrap(),
    );
    registrations
        .put_registration(fcp_core::storage::Registration {
            tenant_id,
            aor: "sip:alice@fcp.local".to_string(),
            contact: "sip:alice@10.0.0.7:5060".to_string(),
            expires_at: SystemTime::now() + Duration::from_secs(600),
            call_id: "reg-1".to_string(),
            cseq: 1,
            user_agent: Some("desk phone".to_string()),
            q: None,
            registered_at: SystemTime::now(),
            path: None,
            flow_token: None,
            source_address: None,
            instance_id: None,
            reg_id: None,
            push: None,
            flow_node: None,
            node_id: None,
        })
        .await
        .unwrap();
    until(
        "the heartbeat never carried the registration",
        cloud.http,
        &cloud.site_path(),
        Some(&cloud.tenant_id),
        |s| {
            s["registrations"]["total_users"] == 1
                && s["registrations"]["users"][0]["aor"] == "sip:alice@fcp.local"
        },
    )
    .await;

    // The site's speech by hour comes up as the site's
    // (docs/TENANT_SPEECH.md S5); counted again in the same hour, the
    // cloud holds the site's count, not the two added.
    {
        use fcp_core::speech::{SpeechUsage, SpeechUsageStore};
        let hour = SystemTime::UNIX_EPOCH
            + Duration::from_secs(
                SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_secs()
                    / 3600
                    * 3600,
            );
        let usage = |characters| SpeechUsage {
            tenant_id,
            provider: "piper".into(),
            hour,
            requests: 1,
            characters,
            synthesized_characters: characters,
            // And what was heard (docs/TENANT_RECOGNITION.md R4).
            recognized_ms: characters * 1000,
            site_id: None,
        };
        async fn cloud_has(
            cloud: &fcp_storage_postgres::PostgresStorage,
            tenant: fcp_core::TenantId,
            site: &str,
            hour: SystemTime,
            want: u64,
        ) -> bool {
            cloud
                .speech_usage(&tenant, hour, hour + Duration::from_secs(3600))
                .await
                .unwrap()
                .iter()
                .any(|u| {
                    u.site_id.as_deref() == Some(site)
                        && u.characters == want
                        && u.recognized_ms == want * 1000
                })
        }
        let site_name = cloud.site_id.clone();
        site.add_speech_usage(&[usage(50)]).await.unwrap();
        eventually(
            "the site's speech reaching the cloud",
            Duration::from_secs(30),
            || cloud_has(&cloud_store, tenant_id, &site_name, hour, 50),
        )
        .await;
        site.add_speech_usage(&[usage(20)]).await.unwrap();
        eventually(
            "the site's hour shipped again",
            Duration::from_secs(30),
            || cloud_has(&cloud_store, tenant_id, &site_name, hour, 70),
        )
        .await;
    }

    // The link's status says what it shipped.
    let (_, text) = http_request("GET", &loopback(cloud.link_health, "/status"), &[], None)
        .await
        .unwrap();
    let status: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert!(
        status["ship"]["shipped"].as_u64().unwrap_or(0) >= 4,
        "the link shipped the record and the rows: {status}"
    );
    assert!(
        status["requests"]["answered"].as_u64().unwrap_or(0) >= 2,
        "the link answered the fetch and the delete: {status}"
    );

    cloud.finish().await;
    site_db.drop_db().await;
}
