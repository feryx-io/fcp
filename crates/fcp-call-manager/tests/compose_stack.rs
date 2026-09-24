//! The compose stack, brought up as the compose file wires it and used
//! (`docs/SHIP_AND_OPERATE.md` phase 4): the events server, the admin,
//! the gateway, the call manager and the conference server start from
//! `deploy/docker-compose/docker-compose.yml`'s own configurations,
//! environment and tokens (`fcp_testkit::compose`), against the test's
//! Postgres and Redis — every service the stack runs by default, each
//! waited for until healthy; a phone registers at the call manager, joins
//! a conference room, is seen on the events server and read back through
//! the gateway. The compose file's ports are used as they are, so this is
//! the one test in this binary.

use fcp_testkit::compose::ComposeStack;
use fcp_testkit::smoke::{http_request, loopback};
use fcp_testkit::{bins, cluster, Phone, Seed, Service, TestDatabase};
use std::net::SocketAddr;
use std::time::Duration;

const REALM: &str = "fcp.local"; // the compose call manager's realm
const ROOM: &str = "8000";
const A_WHILE: Duration = Duration::from_secs(20);

async fn start(
    stack: &ComposeStack,
    name: &str,
    dir: &std::path::Path,
    db: &str,
    redis: &str,
) -> Service {
    let svc = stack.service(name);
    let config = stack.config_for(name, dir, redis);
    let env = stack.env_for(name, db);
    let env_refs: Vec<(&str, &str)> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let bin = bins::path(svc.binary, svc.package);
    Service::start_with(name, bin, &config, &svc.health_url, &env_refs).await
}

/// Stop every service, print what each logged, and fail with `why`.
async fn fail(services: Vec<Service>, why: &str) -> ! {
    for service in services.into_iter().rev() {
        let log = service.stop().await;
        let tail: Vec<&str> = log
            .lines()
            .rev()
            .take(60)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        eprintln!(
            "---- last lines of a service's log ----\n{}",
            tail.join("\n")
        );
    }
    panic!("{why}");
}

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
    let dir = tempfile::tempdir().expect("tempdir");
    let seed = Seed::new(db.url(), REALM).await;
    seed.meeting(ROOM, None, None).await;
    let alice = seed.user("alice", "alice-pw", "1001").await;

    // In the compose file's order: the events server, then the admin and
    // the gateway, then every SIP and media service and the job worker.
    // Not the TTS server (its provider needs the image's Python) and not
    // the registrar profile.
    let mut services = Vec::new();
    for name in [
        "fcp-events-server",
        "fcp-admin",
        "fcp-api-gateway",
        "fcp-call-manager",
        "fcp-conference",
        "fcp-ivr",
        "fcp-voicemail",
        "fcp-presence",
        "fcp-sbc",
        "fcp-media",
        "fcp-webrtc-gw",
        "fcp-jobs",
    ] {
        services.push(start(&stack, name, dir.path(), db.url(), &redis).await);
    }

    let admin_token = stack.dotenv("FCP_ADMIN_AUTH_TOKEN");
    let events_token = stack.dotenv("FCP_EVENTS_AUTH_TOKEN");

    // A phone registers at the call manager (SIP 5060, as the compose
    // file publishes it) and joins the room on the conference server.
    let cm_sip: SocketAddr = "127.0.0.1:5060".parse().unwrap();
    let phone = Phone::new(&alice.username, &alice.password, REALM, cm_sip)
        .await
        .expect("phone binds");
    assert_eq!(phone.register().await.expect("register"), 200);
    let call = phone.call(ROOM).await.expect("INVITE sent");
    if let Err(e) = call.answered(A_WHILE).await {
        fail(
            services,
            &format!("the conference server did not admit the caller: {e}"),
        )
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
        .or_else(|| via_gateway["calls"].as_array().map(|a| a.len()))
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
    for service in services.into_iter().rev() {
        let log = service.stop().await;
        assert!(!log.contains("panicked"), "a service panicked:\n{log}");
    }
    db.drop_db().await;
}
