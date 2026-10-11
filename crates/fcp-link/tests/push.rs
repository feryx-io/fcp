//! A site's pushes go through its cloud (`docs/PUSH.md` §2.4, phase P4b):
//! the site holds none of the publisher's keys. Its call manager's `[push]`
//! lists the apps and relays each push to the site's link, which sends it
//! up its mutual-TLS link; the cloud admin hands it to a call manager with
//! the keys, and what came of it goes back down. A sleeping app at the
//! site is woken this way and answers there; a token the provider no longer
//! knows is taken off the site's binding; the link refuses a push without
//! its token, and the cloud one for another tenant. Gated on
//! `FCP_TEST_DATABASE_URL`; skipped with a notice when unset.

mod common;

use common::{cloud_with_site_opts, CallManagerToml, CloudOptions};
use fcp_core::storage::{
    password::compute_ha1, CredentialStore, DeviceStore, ExtensionStore, RegistrationStore, User,
};
use fcp_core::{Device, DeviceType, Extension, ExtensionTarget, TenantId};
use fcp_storage_postgres::{PostgresConfig, PostgresStorage};
use fcp_testkit::media::allocate_port_pair;
use fcp_testkit::push::{StandInApns, APNS_TEAM, APNS_TOPIC};
use fcp_testkit::smoke::{free_port, http_request, loopback, toml_path};
use fcp_testkit::{bins, Phone, Service, TestDatabase};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

const REALM: &str = "fcp.local";
const PUSH_TOKEN: &str = "site-push-token-2026";
const APP_TOKEN: &str = "a1b2c3d4e5f6a7b8c9d0e1f2";
const RING: Duration = Duration::from_secs(12);

async fn storage(url: &str) -> PostgresStorage {
    PostgresStorage::new(PostgresConfig {
        connection_string: url.to_string(),
        read_replica_url: None,
        max_replica_lag: Duration::from_secs(5),
        max_connections: 4,
        connection_timeout: Duration::from_secs(30),
        encryption: None,
    })
    .await
    .expect("the database")
}

/// A user of the tenant's, made in the cloud as an administrator would,
/// with a softphone on the user's address of record and an extension.
async fn user(cloud: &PostgresStorage, tenant: TenantId, name: &str, password: &str, ext: &str) {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let user: User = serde_json::from_value(serde_json::json!({
        "username": name,
        "password": password,
        "sip_ha1_hash": compute_ha1(name, REALM, password),
        "tenant_id": tenant,
        "display_name": name,
        "extension": ext,
        "enabled": true,
        "created_at": now,
        "updated_at": now,
    }))
    .unwrap();
    let user_id = user.id;
    cloud.put_user(user).await.unwrap();
    let mut device = Device::new(format!("{name} softphone"), DeviceType::Softphone, tenant);
    device.owner_user_id = Some(user_id);
    device.sip_uri = Some(format!("sip:{name}@{REALM}"));
    cloud.put_device(device).await.unwrap();
    cloud
        .put_extension(
            Extension::new(ext.to_string(), ExtensionTarget::User(user_id), tenant).unwrap(),
        )
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_sites_sleeping_app_is_woken_through_the_cloud() {
    let Some(site_db) = TestDatabase::create().await else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping the site push test");
        return;
    };
    let apns = StandInApns::start().await;
    let (cloud_cm_sip, cloud_cm_http) = (free_port(), free_port());
    let (site_sip, site_sips, site_http) = (free_port(), free_port(), free_port());
    let cloud = cloud_with_site_opts(CloudOptions {
        site_database_url: Some(site_db.url()),
        call_manager_url: Some(format!("http://127.0.0.1:{cloud_cm_http}")),
        site_push_token: Some(PUSH_TOKEN.to_string()),
        ..Default::default()
    })
    .await
    .expect("the database was there a moment ago");
    let dir = toml_path(cloud.dir.path());
    let tenant: TenantId = cloud.tenant_id.parse().unwrap();

    // The cloud's call manager holds the publisher's keys.
    let key = cloud.dir.path().join("apns.p8");
    std::fs::write(&key, apns.config().key.unwrap()).unwrap();
    let cloud_config = format!(
        r#"{base}
[[push.apns]]
team_id = "{APNS_TEAM}"
key_id = "{key_id}"
key_file = "{key}"
topics = ["{APNS_TOPIC}"]
endpoint = "{url}"
"#,
        base = CallManagerToml {
            node_id: "cloud-cm",
            sip: cloud_cm_sip,
            http: cloud_cm_http,
            db: cloud.db.url(),
            dir: &dir,
            rtp_start: allocate_port_pair().rtp_port,
            ..Default::default()
        }
        .render(),
        key_id = apns.config().key_id,
        key = toml_path(&key),
        url = apns.config().endpoint.unwrap(),
    );
    let cloud_cm = Service::start_with(
        "fcp-call-manager (cloud)",
        bins::path("fcp-call-manager", "fcp-call-manager"),
        &cloud_config,
        &loopback(cloud_cm_http, "/health"),
        &[],
    )
    .await;

    // Two of the tenant's users, made in the cloud and synced to the site.
    let cloud_store = storage(cloud.db.url()).await;
    user(
        &cloud_store,
        tenant,
        "alice",
        "Alice-at-the-site-2026!",
        "2001",
    )
    .await;
    user(&cloud_store, tenant, "bob", "Bob-at-the-site-2026!", "2002").await;
    let site_store = storage(site_db.url()).await;
    common::eventually("bob reaches the site", Duration::from_secs(20), || {
        let site_store = &site_store;
        async move {
            site_store
                .get_user_in_tenant("bob", &tenant)
                .await
                .ok()
                .flatten()
                .is_some()
                && site_store
                    .get_extension_by_number(&tenant, "2002")
                    .await
                    .ok()
                    .flatten()
                    .is_some()
        }
    })
    .await;

    // The site's call manager: the apps listed without keys, its pushes
    // relayed to the link.
    let site_cert = cloud.ca.node("site-cm");
    let site_config = CallManagerToml {
        node_id: "site-cm",
        sip: site_sip,
        http: site_http,
        db: site_db.url(),
        dir: &dir,
        rtp_start: allocate_port_pair().rtp_port,
        ..Default::default()
    }
    .render()
    .replacen(
        "local_ip = \"127.0.0.1\"\n",
        &format!(
            "local_ip = \"127.0.0.1\"\nsips_bind = \"127.0.0.1:{site_sips}\"\n\
             tls_cert_path = \"{}\"\ntls_key_path = \"{}\"\n",
            toml_path(&site_cert.cert_path),
            toml_path(&site_cert.key_path),
        ),
        1,
    ) + &format!(
        r#"
[push.relay]
url = "http://127.0.0.1:{link}/push"
token = "{PUSH_TOKEN}"

[[push.apns]]
team_id = "{APNS_TEAM}"
topics = ["{APNS_TOPIC}"]
"#,
        link = cloud.link_health,
    );
    let site_cm = Service::start_with(
        "fcp-call-manager (site)",
        bins::path("fcp-call-manager", "fcp-call-manager"),
        &site_config,
        &loopback(site_http, "/health"),
        &[],
    )
    .await;
    // `panic!` with both call managers' logs.
    macro_rules! fail {
        ($($arg:tt)+) => {{
            let mut logs = String::new();
            logs.push_str("\n===== the site's call manager =====\n");
            logs.push_str(&site_cm.kill().await);
            logs.push_str("\n===== the cloud's call manager =====\n");
            logs.push_str(&cloud_cm.kill().await);
            panic!("{}\n{logs}", format!($($arg)+));
        }};
    }

    // Bob's app registers at the site with its push address — the site
    // knows the app though it holds no key for it — then sleeps.
    let site_tls = SocketAddr::from(([127, 0, 0, 1], site_sips));
    let site_udp = SocketAddr::from(([127, 0, 0, 1], site_sip));
    let app = Phone::new_app(
        "bob",
        "Bob-at-the-site-2026!",
        REALM,
        site_tls,
        &cloud.ca.ca_path,
        "<urn:uuid:00000000-0000-0000-0000-0000000000b2>",
    )
    .await
    .unwrap();
    app.set_push(Some(&format!(
        ";pn-provider=apns;pn-param={APNS_TEAM}.{APNS_TOPIC};pn-prid={APP_TOKEN}"
    )))
    .unwrap();
    let code = app.register().await.unwrap();
    if code != 200 {
        fail!("the site refused the app's push binding: {code}");
    }
    app.sleep().await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let app = Arc::new(app);
    app.auto_answer(true);

    // Alice calls Bob at the site; the push goes up the link, the cloud
    // sends it, the app wakes, registers again and answers.
    let mut pushes = apns.subscribe();
    let woken = {
        let app = Arc::clone(&app);
        tokio::spawn(async move {
            let push = StandInApns::next_push(&mut pushes, 15).await?;
            assert_eq!(app.register().await.ok()?, 200);
            Some(push)
        })
    };
    let alice = Phone::new("alice", "Alice-at-the-site-2026!", REALM, site_udp)
        .await
        .unwrap();
    assert_eq!(alice.register().await.unwrap(), 200);
    let call = alice.call("2002").await.unwrap();
    if let Err(e) = call.answered(RING).await {
        fail!("the woken app should answer at the site: {e}");
    }
    let push = woken.await.unwrap().expect("a push woke the app");
    assert_eq!(push.kind(), "call");
    assert_eq!(push.token, APP_TOKEN);
    assert_eq!(
        push.payload["tenant"], cloud.tenant_id,
        "{:?}",
        push.payload
    );
    assert_eq!(call.hangup().await.unwrap(), 200);

    // The link takes no push without its token, and the cloud none for a
    // tenant other than the site's.
    let push_url = loopback(cloud.link_health, "/push");
    let body = |tenant: &str| {
        serde_json::json!({
            "to": {"provider": "apns", "param": format!("{APNS_TEAM}.{APNS_TOPIC}"), "prid": APP_TOKEN},
            "message": {"type": "test", "call": "test-1", "tenant": tenant, "sent": 0},
        })
        .to_string()
    };
    let (code, text) = http_request(
        "POST",
        &push_url,
        &[("Content-Type", "application/json")],
        Some(&body(&cloud.tenant_id)),
    )
    .await
    .unwrap();
    assert_eq!(code, 401, "{text}");
    let bearer = format!("Bearer {PUSH_TOKEN}");
    let headers = [
        ("Content-Type", "application/json"),
        ("Authorization", bearer.as_str()),
    ];
    let (code, text) = http_request(
        "POST",
        &push_url,
        &headers,
        Some(&body(&TenantId::nil().to_string())),
    )
    .await
    .unwrap();
    assert_eq!(code, 403, "{text}");

    // A token Apple no longer knows: the outcome comes back down the link
    // and the site takes the push address off Bob's app's binding.
    app.sleep().await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    apns.forget(APP_TOKEN);
    let call = alice.call("2002").await.unwrap();
    assert!(call.answered(RING).await.is_err(), "nobody can answer");
    let aor = format!("sip:bob@{REALM}");
    let mut let_go = false;
    for _ in 0..40 {
        let rows = site_store
            .get_registrations_in_tenant(&tenant, &aor)
            .await
            .unwrap_or_default();
        if !rows.is_empty() && rows.iter().all(|r| r.push.is_none()) {
            let_go = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    if !let_go {
        fail!("the gone token was not taken off the site's binding");
    }
    // Every request the cloud made of Apple's stand-in was well formed.
    assert!(apns.refused().is_empty(), "{:?}", apns.refused());

    site_cm.stop().await;
    cloud_cm.stop().await;
    cloud.finish().await;
    site_db.drop_db().await;
}
