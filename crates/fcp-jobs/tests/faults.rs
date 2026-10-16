//! Deep testing D5b (`docs/DEEP_TESTING.md`): `fcp-jobs` killed with work
//! in hand. The CDR processor is handed a batch, its database stops
//! answering mid-insert, and it is killed; a new one must store each
//! record once, charge each call once and announce each record.
//! (`cdr_pipeline.rs` kills it idle, with records waiting in the stream.)
//! Skipped without `FCP_TEST_DATABASE_URL` and `FCP_TEST_REDIS_URL`.

use fcp_cdr::{RecordBuilder, TerminationReason};
use fcp_core::billing::{
    BalanceTransactionKind, BalanceTransactionRequest, BillingStore, RatePlan,
};
use fcp_core::storage::CdrKind;
use fcp_core::CallId;
use fcp_storage_redis::cdr_stream::RedisCdrStream;
use fcp_storage_redis::{RedisConfig, RedisStorage};
use fcp_testkit::cut::CutLink;
use fcp_testkit::smoke::{http_get, loopback};
use fcp_testkit::{free_port, Seed, Service, TestDatabase};
use rust_decimal_macros::dec;
use std::collections::BTreeSet;
use std::time::{Duration, SystemTime};

const JOBS: &str = env!("CARGO_BIN_EXE_fcp-jobs");
const RECORDS: usize = 5;
const SOON: Duration = Duration::from_secs(10);

fn jobs_config(db: &str, redis: &str, http: u16) -> String {
    format!(
        r#"
[service]
node_id = "faults-jobs"

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

async fn status(http: u16) -> serde_json::Value {
    let (code, body) = http_get(&loopback(http, "/status")).await.unwrap();
    assert_eq!(code, 200, "{body}");
    serde_json::from_str(&body).unwrap()
}

/// An answered call of thirty seconds to a number the plan rates.
fn record(seed: &Seed) -> fcp_core::storage::CallDetailRecord {
    let call = CallId::new();
    let started = SystemTime::now() - Duration::from_secs(40);
    RecordBuilder::new(CdrKind::Call, call, seed.tenant_id)
        .cdr_id(*call.as_uuid())
        .from("sip:alice@faults.local", None)
        .to("sip:5550100@faults.local", None)
        .started_at(started)
        .answered_at(Some(started + Duration::from_secs(10)))
        .ended_at(Some(started + Duration::from_secs(40)))
        .reason(TerminationReason::hangup_by(
            fcp_core::storage::HangupParty::Caller,
        ))
        .node(Some("faults-node".to_string()), None)
        .build()
}

/// The cdr ids announced as `cdr.written` on the event stream.
async fn announced(redis: &str) -> BTreeSet<String> {
    fcp_testkit::cluster::streamed_events(redis)
        .await
        .into_iter()
        .filter(|e| e["event_type"] == "cdr" && e["type"] == "written")
        .filter_map(|e| e["cdr_id"].as_str().map(str::to_string))
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_processor_killed_holding_a_batch_leaves_it_to_the_next() {
    let (Some(db), Some(redis)) = (
        TestDatabase::create().await,
        fcp_testkit::cluster::private_redis_url(),
    ) else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL or FCP_TEST_REDIS_URL is not set");
        return;
    };
    let seed = Seed::new(db.url(), "faults.local").await;
    let now = SystemTime::now();
    let storage = seed.storage();
    storage
        .create_rate_plan(RatePlan {
            id: uuid::Uuid::new_v4(),
            tenant_id: seed.tenant_id,
            name: "faults".into(),
            currency: "USD".into(),
            default_rate_per_minute: dec!(0.60),
            connect_fee: dec!(0),
            billing_increment_secs: 60,
            is_default: true,
            enabled: true,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
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

    // The processor reaches its database through a link the test holds.
    let (link, through) = CutLink::for_url(db.url()).await;
    let (jobs, http) = start_jobs(&through, &redis).await;

    // The database stops answering; records arrive, and the processor
    // takes them and waits on its insert.
    link.silence();
    let stream = RedisCdrStream::new(
        RedisStorage::new(RedisConfig {
            url: redis.clone(),
            key_prefix: "fcp:".to_string(),
        })
        .await
        .unwrap(),
        "faults-shipper",
        Duration::from_secs(3600),
    );
    let records: Vec<_> = (0..RECORDS).map(|_| record(&seed)).collect();
    stream.append(&records).await.unwrap();
    let deadline = tokio::time::Instant::now() + SOON;
    loop {
        let s = status(http).await;
        if s["cdr"]["source"]["pending"].as_u64() == Some(RECORDS as u64) {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            let logs = jobs.kill().await;
            panic!("the processor never took the batch: {s}\n{logs}");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Killed holding it; the database comes back; a new processor starts.
    let killed = jobs.kill().await;
    link.restore().await;
    let (jobs, http) = start_jobs(&through, &redis).await;

    // Every record stored once.
    let stored = seed
        .cdrs(Some(CdrKind::Call), RECORDS, Duration::from_secs(30))
        .await;
    let ids: BTreeSet<String> = stored.iter().map(|r| r.cdr_id.to_string()).collect();
    let expected: BTreeSet<String> = records.iter().map(|r| r.cdr_id.to_string()).collect();
    assert_eq!(stored.len(), RECORDS, "{stored:#?}\n{killed}");
    assert_eq!(ids, expected, "{killed}");

    // Each call charged once: five minutes at 0.60.
    let deadline = tokio::time::Instant::now() + SOON;
    loop {
        let balance = storage.get_balance(&seed.tenant_id).await.unwrap().unwrap();
        if balance.balance == dec!(7.0) {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "balance {}\n{killed}",
            balance.balance
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let charges = storage
        .list_balance_transactions(&seed.tenant_id, 20, 0)
        .await
        .unwrap()
        .into_iter()
        .filter(|t| t.kind == BalanceTransactionKind::CallCharge)
        .count();
    assert_eq!(charges, RECORDS);

    // Each record announced, and nothing left in hand.
    let deadline = tokio::time::Instant::now() + SOON;
    loop {
        let seen = announced(&redis).await;
        if expected.is_subset(&seen) {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "announced {seen:?}, wanted {expected:?}"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let s = status(http).await;
    assert_eq!(s["cdr"]["source"]["pending"], 0, "{s}");

    jobs.stop().await;
    db.drop_db().await;
}
