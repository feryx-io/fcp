//! A process's log lines reach its service's stream in Redis
//! (`docs/LOG_VIEWER.md` L1): shipped once the beacon starts, tagged with
//! the tenant and call of the span around them, secrets hidden. Gated on
//! `FCP_TEST_REDIS_URL`; skipped with a notice when unset.

use fcp_cluster::logging::{self, LogRecord};
use fcp_cluster::{ServiceBeacon, ServiceIdentity};
use fcp_storage_redis::{RedisConfig, RedisStorage};
use std::time::Duration;

#[tokio::test]
async fn a_line_reaches_its_services_stream_tagged_and_redacted() {
    let Ok(url) = std::env::var("FCP_TEST_REDIS_URL") else {
        eprintln!("FCP_TEST_REDIS_URL not set; skipping the log shipping test");
        return;
    };
    logging::init("info", "compact", true).unwrap();
    let prefix = format!("logs-test-{}:", uuid::Uuid::new_v4().simple());
    let config = RedisConfig {
        url: url.clone(),
        key_prefix: prefix.clone(),
    };
    let beacon = ServiceBeacon::new(ServiceIdentity::new("logs-node-1", "call_manager"))
        .start(config.clone());

    {
        let call = tracing::info_span!(
            "call",
            call_id = "call-1",
            tenant_id = tracing::field::Empty
        );
        let _in = call.enter();
        tracing::info!("before the tenant is known");
        call.record("tenant_id", "tenant-00000000-0000-0000-0000-0000000000aa");
        tracing::info!(
            token = "s3cret",
            "registered with api_key=k-123 from 192.0.2.7"
        );
        tracing::debug!("below the ship level");
    }
    tracing::warn!("a platform line");

    let storage = RedisStorage::new(config).await.unwrap();
    let key = storage.key(&logging::stream_key("logs-node-1"));
    let mut conn = storage.connection();
    let mut records: Vec<LogRecord> = Vec::new();
    for _ in 0..50 {
        let entries: Vec<(String, Vec<(String, String)>)> = redis::cmd("XRANGE")
            .arg(&key)
            .arg("-")
            .arg("+")
            .query_async(&mut conn)
            .await
            .unwrap_or_default();
        records = entries
            .into_iter()
            .flat_map(|(_, f)| f)
            .filter(|(k, _)| k == "r")
            .filter_map(|(_, v)| serde_json::from_str(&v).ok())
            .filter(|r: &LogRecord| r.target.starts_with("logs"))
            .collect();
        if records.len() >= 3 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let find = |text: &str| {
        records
            .iter()
            .find(|r| r.message.contains(text))
            .unwrap_or_else(|| panic!("{text:?} shipped: {records:#?}"))
    };

    let before = find("before the tenant");
    assert_eq!(before.call_id.as_deref(), Some("call-1"));
    assert_eq!(before.tenant_id, None);

    let registered = find("registered with");
    assert_eq!(
        registered.tenant_id.as_deref(),
        Some("00000000-0000-0000-0000-0000000000aa"),
        "the tenant recorded on the span later"
    );
    assert_eq!(
        registered.message,
        "registered with api_key=*** from 192.0.2.7"
    );
    assert_eq!(
        registered.fields.get("token").map(String::as_str),
        Some("***")
    );
    assert_eq!(registered.level, "INFO");

    assert_eq!(find("a platform line").tenant_id, None);
    assert!(records
        .iter()
        .all(|r| !r.message.contains("below the ship")));

    beacon.shutdown().await;
    let _: () = redis::cmd("DEL")
        .arg(&key)
        .query_async(&mut conn)
        .await
        .unwrap();
}
