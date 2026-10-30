//! The compose stack, brought up as the compose file wires it and used
//! (`docs/SHIP_AND_OPERATE.md` phase 4): the events server, the admin,
//! the gateway, the call manager and the conference server start from
//! `deploy/docker-compose/docker-compose.yml`'s own configurations,
//! environment and tokens (`fcp_testkit::compose`), against the test's
//! Postgres and Redis — every service the stack runs by default, each
//! waited for until healthy; a phone registers at the call manager, joins
//! a conference room, is seen on the events server and read back through
//! the gateway, and the call manager's node id reaches it as a `${NAME}`
//! reference resolved from the environment. The certificates the stack's
//! `certs` service would have made come from the test's own CA, and a
//! second phone registers over SIPS with it while the WebRTC gateway and
//! the media server answer over HTTPS; the master key beside them has
//! every service sealing the database's secrets, which the admin's
//! encryption status shows. Every service writes its service beacon
//! (`docs/SERVICES.md`). The compose file's ports are used
//! as they are, so this is the one test in this binary.

use fcp_testkit::compose::ComposeStack;
use fcp_testkit::smoke::{http_request, loopback};
use fcp_testkit::{cluster, Phone, Seed, TestDatabase};
use std::net::SocketAddr;
use std::time::Duration;

const REALM: &str = "fcp.local"; // the compose call manager's realm
const ROOM: &str = "8000";
const NODE_ID_BY_REFERENCE: &str = "cm-by-reference";
const A_WHILE: Duration = Duration::from_secs(20);

async fn get_json(url: &str, token: &str) -> (u16, serde_json::Value) {
    let bearer = format!("Bearer {token}");
    let (code, body) = http_request("GET", url, &[("Authorization", &bearer)], None)
        .await
        .unwrap_or_else(|e| panic!("{url}: {e}"));
    (
        code,
        serde_json::from_str(&body).unwrap_or(serde_json::Value::Null),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_compose_stack_comes_up_and_carries_a_call() {
    let Some(db) = TestDatabase::create().await else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping the compose stack test");
        return;
    };
    let Some(redis) = cluster::private_redis_url() else {
        eprintln!("FCP_TEST_REDIS_URL not set; skipping the compose stack test");
        return;
    };
    let stack = ComposeStack::load();
    let seed = Seed::new(db.url(), REALM).await;
    seed.meeting(ROOM, None, None).await;
    let alice = seed.user("alice", "alice-pw", "1001").await;
    let bob = seed.user("bob", "bob-pw", "1002").await;

    // Every service the compose file runs by default, with its own
    // configuration but for one change: a `${NAME}` in a configuration
    // is resolved from the environment when the file is loaded
    // (`docs/PRODUCTION_STACK.md` phase 2), so the shipped node id
    // becomes a reference, and `/status` below must show what the
    // environment held.
    let local = stack
        .start_local(db.url(), &redis, |name, config, env| {
            if name != "fcp-call-manager" {
                return;
            }
            let referenced = config.replace(
                "node_id = \"cm-docker-01\"",
                "node_id = \"${FCP_STACK_TEST_NODE_ID}\"",
            );
            assert_ne!(
                &referenced, config,
                "the shipped node id is where this test expects it"
            );
            *config = referenced;
            env.push((
                "FCP_STACK_TEST_NODE_ID".to_string(),
                NODE_ID_BY_REFERENCE.to_string(),
            ));
        })
        .await;

    let admin_token = stack.dotenv("FCP_ADMIN_AUTH_TOKEN");
    let events_token = stack.dotenv("FCP_EVENTS_AUTH_TOKEN");
    let cm_token = stack.dotenv("FCP_CALL_MANAGER_API_TOKEN");

    // Encryption at rest is on: every service that opened Postgres did so
    // as `local` under the key in the volume, and the first of them made
    // the initial data encryption key the rest now seal with.
    let (code, encryption) =
        get_json(&loopback(8080, "/api/v1/system/encryption"), &admin_token).await;
    assert_eq!(code, 200, "{encryption}");
    assert_eq!(
        encryption["provider"].as_str(),
        Some("local"),
        "{encryption}"
    );
    assert!(
        encryption["active_key_id"]
            .as_str()
            .is_some_and(|id| !id.is_empty()),
        "an active DEK: {encryption}"
    );

    // The call manager heartbeats into the stack's Redis, and the admin
    // reads it back: the node list has its release, and the cluster runs
    // one release (docs/BACKUP_AND_RESTORE.md §5). Its own `/status`
    // says the same.
    let version = env!("CARGO_PKG_VERSION");
    let (code, listed) = get_json(&loopback(8080, "/api/v1/cluster/nodes"), &admin_token).await;
    assert_eq!(code, 200, "{listed}");
    assert_eq!(listed["available"], true, "{listed}");
    let cm_node = listed["nodes"]
        .as_array()
        .and_then(|nodes| nodes.iter().find(|n| n["node_id"] == NODE_ID_BY_REFERENCE))
        .unwrap_or_else(|| panic!("the call manager is on the node list: {listed}"));
    assert_eq!(cm_node["version"], version, "{cm_node}");
    let (code, stats) = get_json(&loopback(8080, "/api/v1/cluster/stats"), &admin_token).await;
    assert_eq!(code, 200, "{stats}");
    assert_eq!(stats["stats"]["version_skew"], false, "{stats}");
    assert!(
        stats["stats"]["versions"][version]
            .as_array()
            .is_some_and(|nodes| nodes.iter().any(|n| n == NODE_ID_BY_REFERENCE)),
        "the stats group the call manager under this release: {stats}"
    );
    let (code, status) = get_json(&loopback(8084, "/status"), &cm_token).await;
    assert_eq!(code, 200, "{status}");
    assert_eq!(status["cluster_versions"]["skew"], false, "{status}");
    assert!(
        status["cluster_versions"]["versions"][version]
            .as_array()
            .is_some_and(|nodes| nodes.iter().any(|n| n == NODE_ID_BY_REFERENCE)),
        "the call manager's status groups itself under this release: {status}"
    );

    // Every service says it is there, member or not (`docs/SERVICES.md`
    // phase S1): one beacon per process in the stack's Redis, on this
    // release, its required checks passing and the events server reached.
    let beacons = fcp_storage_redis::RedisStorage::new(fcp_storage_redis::RedisConfig {
        url: redis.clone(),
        ..Default::default()
    })
    .await
    .expect("the stack's redis");
    let kinds = [
        "events",
        "admin",
        "api_gateway",
        "call_manager",
        "conference",
        "ivr",
        "voicemail",
        "presence",
        "sbc",
        "media",
        "webrtc_gateway",
        "jobs",
    ];
    let deadline = tokio::time::Instant::now() + A_WHILE;
    let records = loop {
        let records = fcp_cluster::ServiceBeacon::read_all(&beacons)
            .await
            .expect("read the beacons");
        // The events server runs in the stack, so every service that
        // checks it finds it (a check once asked `/ingest/health`).
        let events_reached = records
            .iter()
            .flat_map(|r| &r.checks)
            .all(|c| c.name != "events" || c.ok);
        if kinds.iter().all(|k| records.iter().any(|r| r.kind == *k)) && events_reached {
            break records;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "a beacon from every service, each reaching the events server: {records:?}"
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    };
    for r in &records {
        assert_eq!(r.version, version, "{r:?}");
        assert!(
            r.checks.iter().all(|c| c.ok || !c.required),
            "required checks pass: {r:?}"
        );
    }

    // The call manager runs with the node id its configuration took from
    // the environment through `${FCP_STACK_TEST_NODE_ID}`: `/trunks`
    // reports it.
    let (code, trunks) = get_json(&loopback(8084, "/trunks"), &cm_token).await;
    assert_eq!(code, 200, "{trunks}");
    assert_eq!(
        trunks["node_id"].as_str(),
        Some(NODE_ID_BY_REFERENCE),
        "the node id was resolved from the environment: {trunks}"
    );

    // A phone registers over SIPS (5061, as the compose file publishes
    // it) with the stack's CA: the call manager's certificate is the
    // one the `certs` service would have made, and only that CA is
    // trusted.
    let cm_sips: SocketAddr = "127.0.0.1:5061".parse().unwrap();
    let secure_phone = Phone::new_tls(
        &bob.username,
        &bob.password,
        REALM,
        cm_sips,
        &local.ca.ca_path,
    )
    .await
    .expect("TLS phone binds");
    let registered = secure_phone.register().await;
    if !matches!(registered, Ok(200)) {
        local
            .fail(&format!("REGISTER over SIPS: {registered:?}"))
            .await;
    }

    // The services that serve HTTPS themselves answer under the same CA:
    // the WebRTC gateway on its one listener, the media server beside
    // its HTTP one. (The proxy in front of the admin, gateway and events
    // server is a container this test cannot run.)
    for url in [
        "https://127.0.0.1:8095/health",
        "https://127.0.0.1:8185/health",
    ] {
        let (code, body) = get_json(url, "").await;
        assert_eq!(code, 200, "{url}: {body}");
    }

    // A phone registers at the call manager (SIP 5060, as the compose
    // file publishes it) and joins the room on the conference server.
    let cm_sip: SocketAddr = "127.0.0.1:5060".parse().unwrap();
    let phone = Phone::new(&alice.username, &alice.password, REALM, cm_sip)
        .await
        .expect("phone binds");
    assert_eq!(phone.register().await.expect("register"), 200);
    let call = phone.call(ROOM).await.expect("INVITE sent");
    if let Err(e) = call.answered(A_WHILE).await {
        local
            .fail(&format!(
                "the conference server did not admit the caller: {e}"
            ))
            .await;
    }

    // The events server saw the call: the call manager posted it with the
    // token the compose file gave both.
    let (code, calls) = get_json(&loopback(8081, "/calls"), &events_token).await;
    assert_eq!(code, 200, "{calls}");
    assert!(
        calls["count"].as_u64().unwrap_or(0) >= 1,
        "the events server tracks the call: {calls}"
    );

    // And the gateway reads it back: through the admin, which asks the
    // events server with the token the compose file handed it.
    let (code, via_gateway) = get_json(&loopback(8000, "/api/v1/calls"), &admin_token).await;
    assert_eq!(code, 200, "{via_gateway}");
    let n = via_gateway
        .as_array()
        .map(|a| a.len())
        .or_else(|| via_gateway["data"].as_array().map(|a| a.len()))
        .unwrap_or(0);
    assert!(n >= 1, "the gateway reads the call back: {via_gateway}");

    // The gateway's own view of its backends.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(25);
    loop {
        let (_, status) = get_json(&loopback(8000, "/gateway/status"), "").await;
        if status["backends"]["admin"]["status"] == "healthy"
            && status["backends"]["events"]["status"] == "healthy"
        {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the gateway never saw its backends healthy: {status}"
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    assert_eq!(call.hangup().await.expect("BYE"), 200);

    // Every service stops cleanly and none panicked.
    for (_, service) in local.services.into_iter().rev() {
        let log = service.stop().await;
        assert!(!log.contains("panicked"), "a service panicked:\n{log}");
    }
    db.drop_db().await;
}
