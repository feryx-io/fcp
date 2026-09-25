//! The CDR pipeline end to end (`docs/CDR_PIPELINE.md`, phase 2): a call
//! manager ships its records to the Redis stream, and the processor in
//! `fcp-jobs` rates, stores, settles and announces them. Against the real
//! binaries over real SIP and RTP, on a throwaway database and a Redis
//! logical database of the test's own. Skipped without
//! `FCP_TEST_DATABASE_URL` and `FCP_TEST_REDIS_URL`.

use fcp_core::billing::{
    BalanceTransactionKind, BalanceTransactionRequest, BillingStore, RatePlan,
};
use fcp_testkit::media::allocate_port_pair;
use fcp_testkit::smoke::{http_get, loopback};
use fcp_testkit::{bins, free_port, Phone, Seed, Service, TestDatabase};
use rust_decimal_macros::dec;
use std::net::SocketAddr;
use std::time::{Duration, SystemTime};

const JOBS: &str = env!("CARGO_BIN_EXE_fcp-jobs");
const REALM: &str = "e2e.local";
const SOON: Duration = Duration::from_secs(5);

fn jobs_config(db: &str, redis: &str, http: u16) -> String {
    format!(
        r#"
[service]
node_id = "e2e-jobs"

[redis]
url = "{redis}"

[workers]
count = 1
shutdown_timeout = 5

[health]
enabled = true
bind_address = "127.0.0.1:{http}"

[cdr]
enabled = true
database_url = "{db}"
batch_wait_ms = 50
claim_idle_secs = 1
cache_secs = 1

[logging]
level = "info"
format = "compact"
"#
    )
}

async fn start_jobs(db: &str, redis: &str) -> (Service, u16) {
    let http = free_port();
    let service = Service::start(
        "jobs",
        JOBS,
        &jobs_config(db, redis, http),
        &loopback(http, "/health"),
    )
    .await;
    (service, http)
}

async fn start_call_manager(
    db: &str,
    redis: &str,
    dir: &std::path::Path,
) -> (Service, SocketAddr, u16) {
    let sip = free_port();
    let http = free_port();
    let rtp_start = allocate_port_pair().rtp_port;
    let config = format!(
        r#"
[service]
node_id = "e2e-call-manager"

[sip]
udp_bind = "127.0.0.1:{sip}"
tcp_bind = "127.0.0.1:{sip}"
local_ip = "127.0.0.1"

[registrar]
auth_enabled = true
realm = "{REALM}"

[storage]
database_url = "{db}"
redis_url = "{redis}"

[cdr]
spool_dir = "{dir}/cdr-spool"
sink = "stream"

[http]
bind_address = "127.0.0.1:{http}"

[media]
rtp_port_start = {rtp_start}
rtp_port_end = {rtp_end}
prompts_dir = "{dir}/prompts"
tts_cache_dir = "{dir}/tts-cache"

[logging]
level = "info"
format = "compact"
"#,
        rtp_end = rtp_start + 40,
        dir = dir.display(),
    );
    let bin = bins::path("fcp-call-manager", "fcp-call-manager");
    let service = Service::start("call-manager", bin, &config, &loopback(http, "/health")).await;
    (service, SocketAddr::from(([127, 0, 0, 1], sip)), http)
}

async fn get_json(port: u16, path: &str) -> serde_json::Value {
    let (code, body) = http_get(&loopback(port, path))
        .await
        .unwrap_or_else(|e| panic!("GET {path}: {e}"));
    assert_eq!(code, 200, "GET {path}: {body}");
    serde_json::from_str(&body).unwrap_or_else(|e| panic!("GET {path}: {e}: {body}"))
}

/// The call manager's records, once `n` are there.
/// This node's records — the processor stores whatever is on the stream
/// it shares, and another test's call manager may have left records
/// there, so a record from any other node is not one of ours.
async fn cdrs(http: u16, n: usize, within: Duration) -> Vec<serde_json::Value> {
    let deadline = tokio::time::Instant::now() + within;
    loop {
        let body = get_json(http, "/cdrs").await;
        let list: Vec<serde_json::Value> = body["cdrs"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|r| r["node_id"] == "e2e-call-manager")
            .collect();
        if list.len() >= n {
            return list;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "{} of {n} CDRs after {within:?}: {body}",
            list.len()
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// Alice calls Bob, they talk for a moment, Alice hangs up.
async fn one_call(alice: &Phone, bob: &Phone, extension: &str) {
    let call = alice.call(extension).await.unwrap();
    let answered = bob
        .incoming(SOON)
        .await
        .expect("Bob's phone rings")
        .answer()
        .await
        .unwrap();
    call.answered(SOON).await.unwrap();
    call.media().play_tone(440.0, 800);
    assert!(answered.media().heard(440, SOON).await, "Bob hears Alice");
    assert_eq!(call.hangup().await.unwrap(), 200);
    assert!(answered.ended(SOON).await);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn records_cross_the_stream_to_the_processor_rated_charged_once_and_announced() {
    let (Some(db), Some(redis)) = (
        TestDatabase::create().await,
        fcp_testkit::cluster::private_redis_url(),
    ) else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL or FCP_TEST_REDIS_URL is not set");
        return;
    };
    // The stream is `fcp:cdrs` in whichever Redis database this test
    // was given, and an earlier test binary's call manager may have left
    // records on it (every node writes under the one prefix); the
    // processor would store and charge them here. Start from an empty
    // stream — test binaries run one at a time, so nothing else is
    // writing it now.
    {
        let client = redis::Client::open(redis.as_str()).unwrap();
        let mut conn = client.get_multiplexed_async_connection().await.unwrap();
        let _: () = redis::cmd("DEL")
            .arg("fcp:cdrs")
            .query_async(&mut conn)
            .await
            .unwrap();
    }
    let dir = tempfile::tempdir().unwrap();
    let seed = Seed::new(db.url(), REALM).await;
    let alice_user = seed.user("alice", "alice-pw", "1001").await;
    // A numeric user, so the call's destination has digits to rate.
    let bob_user = seed.user("5550100", "bob-pw", "1002").await;

    // The tenant pays 0.60 a minute, billed per minute, from a balance of 10.
    let now = SystemTime::now();
    let plan: RatePlan = RatePlan {
        id: uuid::Uuid::new_v4(),
        tenant_id: seed.tenant_id,
        name: "e2e".into(),
        currency: "USD".into(),
        default_rate_per_minute: dec!(0.60),
        connect_fee: dec!(0),
        billing_increment_secs: 60,
        is_default: true,
        enabled: true,
        created_at: now,
        updated_at: now,
    };
    let storage = seed.storage();
    storage.create_rate_plan(plan.clone()).await.unwrap();
    storage
        .apply_balance_transaction(BalanceTransactionRequest {
            tenant_id: seed.tenant_id,
            kind: BalanceTransactionKind::TopUp,
            amount: dec!(10),
            currency: "USD".into(),
            call_id: None,
            description: None,
            external_ref: None,
        })
        .await
        .unwrap();

    let (jobs, jobs_http) = start_jobs(db.url(), &redis).await;
    let (call_manager, sip, cm_http) = start_call_manager(db.url(), &redis, dir.path()).await;
    let alice = Phone::new(&alice_user.username, &alice_user.password, REALM, sip)
        .await
        .unwrap();
    let bob = Phone::new(&bob_user.username, &bob_user.password, REALM, sip)
        .await
        .unwrap();
    assert_eq!(alice.register().await.unwrap(), 200);
    assert_eq!(bob.register().await.unwrap(), 200);

    // A call: its record crosses the stream and lands rated.
    one_call(&alice, &bob, &bob_user.extension).await;
    let record = cdrs(cm_http, 1, Duration::from_secs(15)).await.remove(0);
    assert_eq!(record["kind"], "call", "{record}");
    assert_eq!(record["cost"].as_f64(), Some(0.6), "{record}");
    assert_eq!(record["currency"], "USD", "{record}");
    assert_eq!(record["billable_seconds"], 60, "{record}");
    assert_eq!(record["rate_plan_id"], plan.id.to_string(), "{record}");
    assert_eq!(record["direction"], "internal", "{record}");
    assert_eq!(record["node_id"], "e2e-call-manager", "{record}");
    let cdr_id = record["cdr_id"].as_str().unwrap().to_string();

    // Charged once, exactly.
    let deadline = tokio::time::Instant::now() + SOON;
    loop {
        let balance = storage.get_balance(&seed.tenant_id).await.unwrap().unwrap();
        if balance.balance == dec!(9.4) {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "balance {}",
            balance.balance
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Announced on the event stream, and counted.
    let client = redis::Client::open(redis.as_str()).unwrap();
    let mut conn = client.get_multiplexed_async_connection().await.unwrap();
    let deadline = tokio::time::Instant::now() + SOON;
    loop {
        let entries: Vec<(String, Vec<(String, String)>)> = redis::cmd("XRANGE")
            .arg("fcp:event_stream")
            .arg("-")
            .arg("+")
            .query_async(&mut conn)
            .await
            .unwrap_or_default();
        let announced = entries.iter().any(|(_, fields)| {
            fields
                .iter()
                .any(|(_, v)| v.contains("\"cdr_id\"") && v.contains(&cdr_id))
        });
        if announced {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "no cdr.written for {cdr_id}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let (code, metrics) = http_get(&loopback(jobs_http, "/metrics")).await.unwrap();
    assert_eq!(code, 200);
    assert!(
        metrics.contains("cdr_processed_total{node=\"e2e-jobs\",outcome=\"stored\"} 1"),
        "{metrics}"
    );
    let status = get_json(jobs_http, "/status").await;
    assert_eq!(status["cdr"]["source"]["pending"], 0, "{status}");

    // The processor dies. A second call's record waits in the stream, and
    // lands, once, when a processor is back.
    let _ = jobs.kill().await;
    one_call(&alice, &bob, &bob_user.extension).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(
        cdrs(cm_http, 1, SOON).await.len(),
        1,
        "nothing stores it meanwhile"
    );

    let (jobs, _) = start_jobs(db.url(), &redis).await;
    let records = cdrs(cm_http, 2, Duration::from_secs(20)).await;
    assert_eq!(records.len(), 2);
    let deadline = tokio::time::Instant::now() + SOON;
    loop {
        let balance = storage.get_balance(&seed.tenant_id).await.unwrap().unwrap();
        if balance.balance == dec!(8.8) {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "balance {}",
            balance.balance
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let ledger = storage
        .list_balance_transactions(&seed.tenant_id, 10, 0)
        .await
        .unwrap();
    let charges = ledger
        .iter()
        .filter(|t| t.kind == BalanceTransactionKind::CallCharge)
        .count();
    assert_eq!(charges, 2, "{ledger:#?}");

    jobs.stop().await;
    call_manager.stop().await;
    db.drop_db().await;
}
